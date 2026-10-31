//! Boundary-preserving triangulation of validated sampled face charts.
//! Cylindrical interior chords obey the sampling error bound. Exact trims and
//! global solid validity require their separate gates.
use crate::{
    analytic_step::{AnalyticModel, Surface},
    planar_step::{classify, TrimLoop},
    step::{Error, ErrorKind},
    V3,
};
use spade::{
    handles::FixedVertexHandle, AngleLimit, ConstrainedDelaunayTriangulation, HasPosition, Point2,
    RefinementParameters, Triangulation,
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SampleKey {
    Vertex(u64),
    EdgePoint { edge: u64, index: usize },
    FacePoint { face: u64, index: usize },
}
#[derive(Debug, Clone)]
pub struct ChartVertex {
    pub chart: [f64; 2],
    /// Boundary coordinates remain the authoritative source samples; face
    /// interior points are evaluated on the analytic surface.
    pub world: V3,
    pub source: SampleKey,
}
impl HasPosition for ChartVertex {
    type Scalar = f64;
    fn position(&self) -> Point2<f64> {
        Point2::new(self.chart[0], self.chart[1])
    }
}
/// Placeholder for a vertex spade creates during refinement; [`triangulate`] replaces its world coordinates and key.
impl From<Point2<f64>> for ChartVertex {
    fn from(p: Point2<f64>) -> Self {
        Self {
            chart: [p.x, p.y],
            world: [f64::NAN; 3],
            source: SampleKey::FacePoint {
                face: u64::MAX,
                index: usize::MAX,
            },
        }
    }
}
#[derive(Debug, Clone)]
pub struct ChartMesh {
    pub chart_adjustments: Vec<crate::cylindrical_step::ChartAdjustment>,
    pub face_id: u64,
    pub vertices: Vec<ChartVertex>,
    /// Counterclockwise in the analytic chart, before source face sense.
    pub triangles: Vec<[usize; 3]>,
}
/// Delaunay refinement of a face chart's interior (boundary samples untouched).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartQuality {
    pub min_angle_deg: f64,
    /// Interior points allowed per face.
    pub max_points: usize,
}
fn fail(kind: ErrorKind, s: impl Into<String>) -> Error {
    Error {
        offset: 0,
        kind,
        detail: s.into(),
    }
}
fn edge_key(model: &AnalyticModel, edge: u64, index: usize) -> SampleKey {
    let e = &model.body().edges[&edge];
    let n = model.edge_points(edge).unwrap().len();
    if index == 0 {
        SampleKey::Vertex(e.start)
    } else if index == n - 1 {
        SampleKey::Vertex(e.end)
    } else {
        SampleKey::EdgePoint { edge, index }
    }
}
fn regions(
    cdt: &ConstrainedDelaunayTriangulation<ChartVertex>,
    max_work: usize,
) -> Result<Vec<Option<bool>>, Error> {
    // Classify connected triangle regions by crossing constraint parity from
    // the outer face. Rounded centroids can land on a boundary of a very thin
    // triangle; topological propagation never needs such a floating test point.
    if cdt.num_undirected_edges().saturating_mul(2) > max_work {
        return Err(fail(
            ErrorKind::Limit,
            "chart region traversal budget exceeded",
        ));
    }
    let mut adjacent = vec![Vec::new(); cdt.num_all_faces()];
    for e in cdt.undirected_edges() {
        let d = e.as_directed();
        let a = d.face().index();
        let b = d.rev().face().index();
        adjacent[a].push((b, e.is_constraint_edge()));
        adjacent[b].push((a, e.is_constraint_edge()));
    }
    let outer = cdt.outer_face().index();
    let mut parity = vec![None; adjacent.len()];
    parity[outer] = Some(false);
    let mut pending = vec![outer];
    while let Some(a) = pending.pop() {
        for &(b, crossing) in &adjacent[a] {
            let next = parity[a].unwrap() ^ crossing;
            if let Some(previous) = parity[b] {
                if previous != next {
                    return Err(fail(
                        ErrorKind::Topology,
                        "inconsistent constrained chart regions",
                    ));
                }
            } else {
                parity[b] = Some(next);
                pending.push(b);
            }
        }
    }
    if parity.iter().any(Option::is_none) {
        return Err(fail(ErrorKind::Topology, "unreachable chart region"));
    }
    Ok(parity)
}
fn build_cdt(
    loops: &[TrimLoop],
    vertices: &[ChartVertex],
    max_work: usize,
) -> Result<BuiltChart, Error> {
    // Revalidate inputs even when this internal routine is reused by new callers.
    let mut checked = loops.to_vec();
    classify(&mut checked, max_work)?;
    let mut cdt = ConstrainedDelaunayTriangulation::<ChartVertex>::new();
    let mut handles = Vec::new();
    for vertex in vertices {
        let before = cdt.num_vertices();
        let h = cdt
            .insert(vertex.clone())
            .map_err(|e| fail(ErrorKind::Geometry, format!("chart insertion: {e:?}")))?;
        if cdt.num_vertices() != before + 1 {
            return Err(fail(
                ErrorKind::Geometry,
                "distinct boundary samples have identical chart coordinates",
            ));
        }
        handles.push(h);
    }
    let mut constraints = BTreeSet::new();
    let mut offset = 0;
    for l in loops {
        for i in 0..l.points.len() {
            let a = offset + i;
            let b = offset + (i + 1) % l.points.len();
            if !cdt.can_add_constraint(handles[a], handles[b]) {
                return Err(fail(
                    ErrorKind::Geometry,
                    "unrecoverable sampled boundary constraint",
                ));
            }
            cdt.add_constraint(handles[a], handles[b]);
            constraints.insert((a.min(b), a.max(b)));
        }
        offset += l.points.len();
    }
    Ok((cdt, handles, constraints))
}
#[derive(Clone, Copy)]
struct Shaping {
    quality: Option<ChartQuality>,
    split_boundary: bool,
}
type BuiltChart = (
    ConstrainedDelaunayTriangulation<ChartVertex>,
    Vec<FixedVertexHandle>,
    BTreeSet<(usize, usize)>,
);
struct ShapedChart {
    cdt: ConstrainedDelaunayTriangulation<ChartVertex>,
    vertices: Vec<ChartVertex>,
    handles: Vec<FixedVertexHandle>,
    constraints: BTreeSet<(usize, usize)>,
}
fn chart_midpoint(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    let p = spade::mitigate_underflow(Point2::new(
        a[0] / 2. + b[0] / 2.,
        a[1] / 2. + b[1] / 2.,
    ));
    [p.x, p.y]
}
/// The face chart's constrained triangulation with its interior points: the chord rule on cylinders and, when asked for,
/// Delaunay quality refinement. `split_boundary` lets the refinement split boundary segments (the new points are then on the
/// boundary and the result is only good for reading off [`BoundarySplit`]s); otherwise the boundary is kept exactly.
fn shape_chart(
    face_id: u64,
    loops: &[TrimLoop],
    mut vertices: Vec<ChartVertex>,
    max_work: usize,
    surface: Option<&Surface>,
    chord_mm: f64,
    shaping: Shaping,
) -> Result<ShapedChart, Error> {
    let Shaping {
        quality,
        split_boundary,
    } = shaping;
    let (mut cdt, mut handles, constraints) = build_cdt(loops, &vertices, max_work)?;
    // Interior quality points: the boundary polygon is kept exactly, so neighbouring faces still agree on every shared sample,
    // and the points are evaluated on the exact surface.
    let boundary_count = vertices.len();
    // Grading exports only new constraint-chain vertices. Repair a mapped
    // inversion anchored to a coarse boundary chord before interior quality
    // points can hide it in this temporary CDT.
    if let Some(surface) = surface.filter(|s| split_boundary && matches!(s, Surface::Torus { .. })) {
        let mut work = 0usize;
        loop {
            let parity = regions(&cdt, max_work.saturating_sub(work))?;
            let candidate = cdt.inner_faces().find_map(|face| {
                if parity[face.index()] != Some(true) {
                    return None;
                }
                let face_vertices = face.vertices().map(|v| v.data().clone());
                let center = std::array::from_fn(|axis| {
                    face_vertices.iter().map(|v| v.chart[axis] / 3.).sum()
                });
                let normal = surface.normal(center)?;
                let [a, b, c] = face_vertices.each_ref().map(|v| v.world);
                let orientation =
                    crate::dot(crate::cross(crate::sub(b, a), crate::sub(c, a)), normal);
                let edge_scale = [
                    crate::dot(crate::sub(b, a), crate::sub(b, a)),
                    crate::dot(crate::sub(c, b), crate::sub(c, b)),
                    crate::dot(crate::sub(a, c), crate::sub(a, c)),
                ]
                .into_iter()
                .fold(0., f64::max);
                // The repair is for a genuinely reversed coarse chord, not
                // roundoff in nearly coincident chart samples. The final
                // sampled-face gate still decides whether such a face is valid.
                if orientation.is_finite() && orientation >= -1e-12 * edge_scale {
                    return None;
                }
                let edge = face
                    .adjacent_edges()
                    .into_iter()
                    .filter(|e| e.is_constraint_edge())
                    .filter(|e| {
                        let [a, b] = e.vertices();
                        (0..2).any(|k| {
                            let middle = a.data().chart[k] / 2. + b.data().chart[k] / 2.;
                            middle != a.data().chart[k] && middle != b.data().chart[k]
                        })
                    })
                    .max_by(|a, b| a.length_2().total_cmp(&b.length_2()))?;
                let [a, b] = edge.vertices();
                Some((
                    edge.as_undirected().fix(),
                    [a.fix(), b.fix()],
                    a.data().chart,
                    b.data().chart,
                ))
            });
            let Some((edge, [a, b], ca, cb)) = candidate else {
                break;
            };
            work = work
                .checked_add(1)
                .filter(|n| *n <= max_work)
                .ok_or_else(|| {
                    fail(
                        ErrorKind::Limit,
                        "curved boundary orientation budget exceeded",
                    )
                })?;
            let chart = chart_midpoint(ca, cb);
            let world = surface
                .point(chart)
                .ok_or_else(|| fail(ErrorKind::Geometry, "unresolved curved boundary point"))?;
            let vertex = ChartVertex {
                chart,
                world,
                source: SampleKey::FacePoint {
                    face: face_id,
                    index: vertices.len() - boundary_count,
                },
            };
            cdt.remove_constraint_edge(edge);
            let before = cdt.num_vertices();
            let handle = cdt.insert(vertex.clone()).map_err(|e| {
                fail(
                    ErrorKind::Geometry,
                    format!("curved boundary insertion: {e:?}"),
                )
            })?;
            let inserted = cdt.num_vertices() == before + 1;
            if !inserted && (handle == a || handle == b) {
                return Err(fail(
                    ErrorKind::Geometry,
                    format!("face #{face_id} curved boundary midpoint is unresolved: {ca:?} to {cb:?}, midpoint {chart:?}"),
                ));
            }
            cdt.add_constraint(a, handle);
            cdt.add_constraint(handle, b);
            if inserted {
                vertices.push(vertex);
                handles.push(handle);
            }
        }
    }
    // On a cylinder the chord rule below adds midpoints that the Delaunay flips then reshape, so the two alternate until
    // neither adds anything.
    for _pass in 0..8 {
        let start = vertices.len();
        if let (Some(q), Some(surface)) = (quality, surface) {
            let mut parameters = RefinementParameters::<f64>::new()
                .with_angle_limit(AngleLimit::from_deg(q.min_angle_deg))
                .exclude_outer_faces(true)
                .with_max_additional_vertices(q.max_points);
            if !split_boundary {
                parameters = parameters.keep_constraint_edges();
            }
            let result = cdt.refine(parameters);
            let _ = result.refinement_complete;
            for i in vertices.len()..cdt.num_vertices() {
                let handle = FixedVertexHandle::from_index(i);
                let chart = cdt.vertex(handle).data().chart;
                let world = surface
                    .point(chart)
                    .ok_or_else(|| fail(ErrorKind::Geometry, "unresolved quality point"))?;
                let vertex = ChartVertex {
                    chart,
                    world,
                    source: SampleKey::FacePoint {
                        face: face_id,
                        index: vertices.len() - boundary_count,
                    },
                };
                *cdt.vertex_data_mut(handle) = vertex.clone();
                vertices.push(vertex);
                handles.push(handle);
            }
        }
        // A shallow cylindrical strip can have all four boundary corners on the
        // neighbouring plane. Its unsplit internal chord would then overlap that
        // face despite correct topological boundary recovery. Split curved interior
        // boundary-to-boundary chords on the evaluated surface, preserving every
        // constrained boundary segment and sharing the new point across both sides.
        if let Some(surface) = surface.filter(|s| !matches!(s, Surface::Plane(_))) {
            let mut work = 0usize;
            loop {
                let parity = regions(&cdt, max_work.saturating_sub(work))?;
                work = work
                    .checked_add(cdt.num_undirected_edges().saturating_mul(2))
                    .ok_or_else(|| fail(ErrorKind::Limit, "curved chart work overflow"))?;
                let mut candidates = Vec::new();
                for edge in cdt.undirected_edges() {
                    work = work
                        .checked_add(1)
                        .filter(|n| *n <= max_work)
                        .ok_or_else(|| {
                            fail(ErrorKind::Limit, "curved chart refinement budget exceeded")
                        })?;
                    let directed = edge.as_directed();
                    if edge.is_constraint_edge()
                        || parity[directed.face().index()] != Some(true)
                        || parity[directed.rev().face().index()] != Some(true)
                    {
                        continue;
                    }
                    let [a, b] = edge.vertices();
                    let boundary_diagonal =
                        a.index() < boundary_count && b.index() < boundary_count;
                    let a = a.data();
                    let b = b.data();
                    // The cylinder keeps its closed-form bound on the circumferential span; the other curved surfaces
                    // use the sagitta itself: the chart midpoint's surface point against the chord midpoint.
                    let span = (a.chart[0] - b.chart[0]).abs();
                    let cylinder = match surface {
                        Surface::Cylinder { radius, .. } => Some(*radius),
                        _ => None,
                    };
                    let bound =
                        cylinder.map(|r| r * crate::analytic_step::chord_angle(r, chord_mm));
                    if let Some(bound) = bound {
                        if !boundary_diagonal && span <= bound {
                            continue;
                        }
                        if a.chart[0] == b.chart[0] {
                            continue;
                        }
                    }
                    let chart = chart_midpoint(a.chart, b.chart);
                    let world = surface.point(chart).ok_or_else(|| {
                        fail(ErrorKind::Geometry, "unresolved curved interior point")
                    })?;
                    let chord: V3 = std::array::from_fn(|i| a.world[i] / 2. + b.world[i] / 2.);
                    if bound.is_none() {
                        let sag = ((world[0] - chord[0]).powi(2)
                            + (world[1] - chord[1]).powi(2)
                            + (world[2] - chord[2]).powi(2))
                        .sqrt();
                        if !boundary_diagonal && sag <= chord_mm {
                            continue;
                        }
                    }
                    if world != chord {
                        candidates.push(ChartVertex {
                            chart,
                            world,
                            source: SampleKey::FacePoint {
                                face: face_id,
                                index: vertices.len() - boundary_count,
                            },
                        });
                    } else if bound.is_some_and(|bound| span > bound) {
                        return Err(fail(
                            ErrorKind::Geometry,
                            "curved error bound is below coordinate precision",
                        ));
                    }
                }
                if candidates.is_empty() {
                    break;
                }
                // Reclassify once per batch, not once per inserted vertex. Constraint
                // boundaries stay fixed, so all admitted interior points remain in
                // the same polygonal domain after local Delaunay edge flips.
                for mut vertex in candidates {
                    vertex.source = SampleKey::FacePoint {
                        face: face_id,
                        index: vertices.len() - boundary_count,
                    };
                    let before = cdt.num_vertices();
                    let handle = cdt.insert(vertex.clone()).map_err(|e| {
                        fail(
                            ErrorKind::Geometry,
                            format!("curved interior insertion: {e:?}"),
                        )
                    })?;
                    if cdt.num_vertices() != before + 1 {
                        return Err(fail(
                            ErrorKind::Geometry,
                            "curved chord midpoint is unresolved",
                        ));
                    }
                    vertices.push(vertex);
                    handles.push(handle);
                }
            }
        }
        if vertices.len() == start {
            break;
        }
    }
    Ok(ShapedChart {
        cdt,
        vertices,
        handles,
        constraints,
    })
}
fn triangulate(
    face_id: u64,
    loops: &[TrimLoop],
    vertices: Vec<ChartVertex>,
    max_work: usize,
    surface: Option<&Surface>,
    chord_mm: f64,
    quality: Option<ChartQuality>,
) -> Result<ChartMesh, Error> {
    let ShapedChart {
        cdt,
        vertices,
        handles,
        constraints,
    } = shape_chart(
        face_id,
        loops,
        vertices,
        max_work,
        surface,
        chord_mm,
        Shaping {
            quality,
            split_boundary: false,
        },
    )?;
    let parity = regions(&cdt, max_work)?;
    let mut triangles = Vec::new();
    for f in cdt.inner_faces() {
        if parity[f.index()] == Some(true) {
            triangles.push(f.vertices().map(|v| v.index()));
        }
    }
    if triangles.is_empty() {
        return Err(fail(ErrorKind::Geometry, "empty chart triangulation"));
    }
    let mut incidence = BTreeMap::<(usize, usize), usize>::new();
    let mut used = BTreeSet::<usize>::new();
    for t in &triangles {
        used.extend(t.iter().copied());
        for i in 0..3 {
            let a = t[i];
            let b = t[(i + 1) % 3];
            *incidence.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    if used.len() != vertices.len()
        || incidence.iter().any(|(e, n)| {
            if constraints.contains(e) {
                *n != 1
            } else {
                *n != 2
            }
        })
        || constraints.iter().any(|e| incidence.get(e) != Some(&1))
    {
        return Err(fail(
            ErrorKind::Topology,
            format!("face #{face_id} chart boundary mismatch: {} used / {} vertices; invalid edges {:?}",used.len(),vertices.len(),incidence.iter().filter(|(e,n)|if constraints.contains(e){**n!=1}else{**n!=2}).take(5).collect::<Vec<_>>()),
        ));
    }
    // Incremental insertion creates contiguous stable vertex indices; verify the
    // mapping explicitly instead of assuming dependency storage semantics.
    for (i, h) in handles.iter().enumerate() {
        if h.index() != i {
            return Err(fail(
                ErrorKind::Topology,
                "unexpected CDT vertex reindexing",
            ));
        }
    }
    Ok(ChartMesh {
        chart_adjustments: Vec::new(),
        face_id,
        vertices,
        triangles,
    })
}
impl AnalyticModel {
    pub fn triangulate_sampled_chart(
        &self,
        face_id: u64,
        max_work: usize,
    ) -> Result<ChartMesh, Error> {
        self.triangulate_sampled_chart_with_policy(
            face_id,
            max_work,
            crate::cylindrical_step::ChartPolicy::Strict,
        )
    }
    pub fn triangulate_sampled_chart_with_policy(
        &self,
        face_id: u64,
        max_work: usize,
        policy: crate::cylindrical_step::ChartPolicy,
    ) -> Result<ChartMesh, Error> {
        self.triangulate_sampled_chart_with_quality(face_id, max_work, policy, None)
    }
    /// [`Self::triangulate_sampled_chart_with_policy`] plus optional interior quality refinement ([`ChartQuality`]).
    pub fn triangulate_sampled_chart_with_quality(
        &self,
        face_id: u64,
        max_work: usize,
        policy: crate::cylindrical_step::ChartPolicy,
        quality: Option<ChartQuality>,
    ) -> Result<ChartMesh, Error> {
        let inputs = self.chart_inputs(face_id, max_work, policy)?;
        let mut mesh = triangulate(
            face_id,
            &inputs.loops,
            inputs.vertices,
            max_work,
            inputs.surface.as_ref(),
            self.chord_tolerance_mm(),
            quality,
        )?;
        mesh.chart_adjustments = inputs.adjustments;
        Ok(mesh)
    }
    fn chart_inputs(
        &self,
        face_id: u64,
        max_work: usize,
        policy: crate::cylindrical_step::ChartPolicy,
    ) -> Result<ChartInputs, Error> {
        let mut centre = None;
        let (mut loops, adjustments) = match self.surface(face_id) {
            Some(Surface::Plane(_)) => (
                self.sampled_planar_trims(face_id, max_work)?.loops,
                Vec::new(),
            ),
            Some(Surface::Cylinder { .. }) => {
                let trims = match policy {
                    crate::cylindrical_step::ChartPolicy::Strict => {
                        self.sampled_cylindrical_trims(face_id, max_work)?
                    }
                    crate::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance => {
                        self.canonicalized_cylindrical_trims(face_id, max_work)?
                    }
                };
                (trims.loops, trims.adjustments)
            }
            Some(Surface::Sphere { .. } | Surface::Param(_)) => (
                self.sampled_planar_trims(face_id, max_work)?.loops,
                Vec::new(),
            ),
            Some(Surface::Cone { .. } | Surface::Torus { .. }) => {
                let trims = self.sampled_periodic_trims(face_id, max_work)?;
                centre = Some(trims.centre);
                (vec![trims.boundary], Vec::new())
            }
            None => return Err(fail(ErrorKind::Geometry, "unknown face")),
        };
        // A chart value below the exact predicates' envelope (1e-30) is rounding noise around zero, not a coordinate: the
        // sampling of an edge point can leave one (observed 8.7e-32 on a cylinder seam).
        for l in &mut loops {
            for p in &mut l.points {
                for x in p {
                    if x.abs() < 1e-25 {
                        *x = 0.;
                    }
                }
            }
        }
        let face = &self.body().faces[&face_id];
        let mut vertices = Vec::new();
        let mut segments = Vec::new();
        for (bound, l) in face.bounds.iter().zip(&loops) {
            let mut samples = Vec::new();
            // The edge segment from each sample to the next one of the bound, in traversal order.
            let mut outgoing = Vec::new();
            let surface = self.surface(face_id).unwrap();
            for usage in crate::planar_step::chart_usages(surface, bound) {
                let p = self.edge_points(usage.edge).unwrap();
                if samples.len().saturating_add(p.len() - 1) > max_work {
                    return Err(fail(
                        ErrorKind::Limit,
                        "chart sample expansion budget exceeded",
                    ));
                }
                for j in 0..p.len() - 1 {
                    let i = if usage.forward { j } else { p.len() - 1 - j };
                    samples.push((p[i], edge_key(self, usage.edge, i)));
                    outgoing.push(SegmentRef {
                        edge: usage.edge,
                        lower: if usage.forward { j } else { p.len() - 2 - j },
                    });
                }
            }
            if !bound.forward {
                samples.reverse();
                // Reversed, the sample k leaves along the segment that used to arrive at it.
                outgoing.reverse();
                let n = outgoing.len();
                outgoing.rotate_left(1 % n.max(1));
            }
            if samples.len() != l.points.len() {
                return Err(fail(
                    ErrorKind::Topology,
                    "chart/source sample count mismatch",
                ));
            }
            for ((world, source), chart) in samples.into_iter().zip(&l.points) {
                vertices.push(ChartVertex {
                    world,
                    source,
                    chart: *chart,
                });
            }
            segments.extend(outgoing);
        }
        let surface = self.surface(face_id).map(|s| match centre {
            Some(c) => s.with_centre(c),
            None => s.clone(),
        });
        Ok(ChartInputs {
            loops,
            adjustments,
            vertices,
            segments,
            surface,
        })
    }
    /// Where a Delaunay refinement of the face chart (with its boundary segments allowed to split) would put new samples on the
    /// face's boundary edges. The shared edges are not changed here: grading them for every face is [`Self::graded`].
    pub(crate) fn boundary_splits(
        &self,
        face_id: u64,
        max_work: usize,
        policy: crate::cylindrical_step::ChartPolicy,
        min_angle_deg: f64,
        floor_mm: f64,
        max_points: usize,
    ) -> Result<Vec<BoundarySplit>, Error> {
        let inputs = self.chart_inputs(face_id, max_work, policy)?;
        let boundary_count = inputs.vertices.len();
        let ShapedChart { cdt, .. } = shape_chart(
            face_id,
            &inputs.loops,
            inputs.vertices.clone(),
            max_work,
            inputs.surface.as_ref(),
            self.chord_tolerance_mm(),
            Shaping {
                quality: Some(ChartQuality {
                    min_angle_deg,
                    max_points,
                }),
                split_boundary: true,
            },
        )?;
        // Loop membership of every original sample.
        let mut loop_of = Vec::with_capacity(boundary_count);
        let mut first_of_loop = Vec::new();
        for (k, l) in inputs.loops.iter().enumerate() {
            first_of_loop.push(loop_of.len());
            loop_of.extend(std::iter::repeat_n(k, l.points.len()));
        }
        let next = |i: usize| {
            let k = loop_of[i];
            let (start, len) = (first_of_loop[k], inputs.loops[k].points.len());
            start + (i - start + 1) % len
        };
        let mut out = Vec::new();
        for i in boundary_count..cdt.num_vertices() {
            let h = FixedVertexHandle::from_index(i);
            let v = cdt.vertex(h);
            let constrained: Vec<_> = v
                .out_edges()
                .filter(|e| e.is_constraint_edge())
                .map(|e| e.to().fix())
                .collect();
            if constrained.len() != 2 {
                continue;
            }
            // Walk along the constrained chain to the original samples on both sides.
            let mut ends = [0usize; 2];
            for (side, first) in constrained.iter().enumerate() {
                let (mut prev, mut cur) = (h, *first);
                while cur.index() >= boundary_count {
                    let nxt = cdt
                        .vertex(cur)
                        .out_edges()
                        .filter(|e| e.is_constraint_edge())
                        .map(|e| e.to().fix())
                        .find(|n| *n != prev);
                    let Some(nxt) = nxt else { break };
                    prev = cur;
                    cur = nxt;
                }
                ends[side] = cur.index();
            }
            let [mut a, mut b] = ends;
            if a >= boundary_count || b >= boundary_count {
                continue;
            }
            if next(a) != b {
                std::mem::swap(&mut a, &mut b);
            }
            if next(a) != b {
                continue;
            }
            let (pa, pb) = (inputs.vertices[a].chart, inputs.vertices[b].chart);
            let length = ((pb[0] - pa[0]).powi(2) + (pb[1] - pa[1]).powi(2)).sqrt();
            if length < 2. * floor_mm || length == 0. {
                continue;
            }
            let seg = inputs.segments[a];
            out.push(BoundarySplit {
                edge: seg.edge,
                segment: seg.lower,
            });
        }
        Ok(out)
    }
}
/// A segment (between edge samples `segment` and `segment + 1`) of `edge` that a face chart's refinement split. Only the
/// segment is reported: where the chart put its point is not used (see [`AnalyticModel::graded`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoundarySplit {
    pub edge: u64,
    pub segment: usize,
}
#[derive(Debug, Clone, Copy)]
struct SegmentRef {
    edge: u64,
    lower: usize,
}
struct ChartInputs {
    loops: Vec<TrimLoop>,
    adjustments: Vec<crate::cylindrical_step::ChartAdjustment>,
    vertices: Vec<ChartVertex>,
    /// Outgoing edge segment of every boundary sample, parallel to `vertices`.
    segments: Vec<SegmentRef>,
    /// The face's surface with its chart branch fixed (a cone's centre angle).
    surface: Option<Surface>,
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytic_step::Frame;

    #[test]
    fn curved_midpoint_stays_in_spades_coordinate_range() {
        let (a, b) = ([-1.0e-27, 0.], [1.0000000000000002e-27, 1.]);
        assert!(spade::validate_coordinate(a[0] / 2. + b[0] / 2.).is_err());
        let p = chart_midpoint(a, b);
        assert_eq!(p[0], 0.);
        assert!(spade::validate_coordinate(p[0]).is_ok());
    }

    #[test]
    fn square_with_hole_preserves_every_boundary_segment_and_area() {
        let loops = vec![
            TrimLoop {
                bound_id: 1,
                points: vec![[0., 0.], [4., 0.], [4., 4.], [0., 4.]],
                is_outer: true,
            },
            TrimLoop {
                bound_id: 2,
                points: vec![[1., 1.], [3., 1.], [3., 3.], [1., 3.]],
                is_outer: false,
            },
        ];
        let v = loops
            .iter()
            .flat_map(|l| &l.points)
            .enumerate()
            .map(|(i, p)| ChartVertex {
                chart: *p,
                world: [p[0], p[1], 0.],
                source: SampleKey::Vertex(i as u64),
            })
            .collect();
        let mesh = triangulate(0, &loops, v, 10000, None, 0.05, None).unwrap();
        let area: f64 = mesh
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.vertices[i].chart);
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.
            })
            .sum();
        assert_eq!(area, 12.);
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.triangles.len(), 8);
    }

    #[test]
    fn torus_boundary_triangle_is_refined_before_its_world_chords_invert() {
        let surface = Surface::Torus {
            frame: Frame {
                origin: [0., 0., 0.],
                x: [1., 0., 0.],
                y: [0., 1., 0.],
                z: [0., 0., 1.],
            },
            major: 0.5,
            minor: 0.1,
        };
        // Reduced from ABC face #339: counterclockwise in the conformal chart,
        // but its three coarse boundary chords form an inward-facing facet.
        let points = vec![
            [4.26359002987186, 0.5015071930442013],
            [4.487989505128274, 0.45898794349181227],
            [4.712388980384689, 0.5015071930442018],
        ];
        let loops = vec![TrimLoop {
            bound_id: 1,
            points: points.clone(),
            is_outer: true,
        }];
        let vertices = points
            .iter()
            .enumerate()
            .map(|(i, &chart)| ChartVertex {
                chart,
                world: surface.point(chart).unwrap(),
                source: SampleKey::Vertex(i as u64),
            })
            .collect();

        let shaped = shape_chart(
            339,
            &loops,
            vertices,
            10_000,
            Some(&surface),
            0.05,
            Shaping {
                quality: None,
                split_boundary: true,
            },
        )
        .unwrap();

        assert!(
            shaped.vertices.len() > 3,
            "the inverted boundary chord was not split"
        );
        let parity = regions(&shaped.cdt, 10_000).unwrap();
        for face in shaped
            .cdt
            .inner_faces()
            .filter(|f| parity[f.index()] == Some(true))
        {
            let triangle = face.vertices().map(|v| v.index());
            let [a, b, c] = triangle.map(|i| shaped.vertices[i].world);
            let center = std::array::from_fn(|axis| {
                triangle
                    .iter()
                    .map(|&i| shaped.vertices[i].chart[axis] / 3.)
                    .sum()
            });
            let normal = surface.normal(center).unwrap();
            assert!(
                crate::dot(crate::cross(crate::sub(b, a), crate::sub(c, a)), normal) > 0.,
                "mapped triangle is locally inverted: {triangle:?}"
            );
        }
    }
}
