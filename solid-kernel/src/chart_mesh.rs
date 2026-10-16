//! Boundary-preserving triangulation of validated sampled face charts.
//! Cylindrical interior chords obey the sampling error bound. Exact trims and
//! global solid validity require their separate gates.
use crate::{
    analytic_step::{AnalyticModel, Surface},
    planar_step::{classify, TrimLoop},
    step::{Error, ErrorKind},
    V3,
};
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Point2, Triangulation};
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
#[derive(Debug, Clone)]
pub struct ChartMesh {
    pub chart_adjustments: Vec<crate::cylindrical_step::ChartAdjustment>,
    pub face_id: u64,
    pub vertices: Vec<ChartVertex>,
    /// Counterclockwise in the analytic chart, before source face sense.
    pub triangles: Vec<[usize; 3]>,
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
fn triangulate(
    face_id: u64,
    loops: &[TrimLoop],
    mut vertices: Vec<ChartVertex>,
    max_work: usize,
    surface: Option<&Surface>,
    chord_mm: f64,
) -> Result<ChartMesh, Error> {
    // Revalidate inputs even when this internal routine is reused by new callers.
    let mut checked = loops.to_vec();
    classify(&mut checked, max_work)?;
    let mut cdt = ConstrainedDelaunayTriangulation::<ChartVertex>::new();
    let mut handles = Vec::new();
    for vertex in &vertices {
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
    // A shallow cylindrical strip can have all four boundary corners on the
    // neighbouring plane. Its unsplit internal chord would then overlap that
    // face despite correct topological boundary recovery. Split curved interior
    // boundary-to-boundary chords on the evaluated surface, preserving every
    // constrained boundary segment and sharing the new point across both sides.
    if let Some(surface @ Surface::Cylinder { radius, .. }) = surface {
        let boundary_count = vertices.len();
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
                let boundary_diagonal = a.index() < boundary_count && b.index() < boundary_count;
                let a = a.data();
                let b = b.data();
                let span = (a.chart[0] - b.chart[0]).abs();
                if !boundary_diagonal
                    && span <= radius * crate::analytic_step::chord_angle(*radius, chord_mm)
                {
                    continue;
                }
                if a.chart[0] == b.chart[0] {
                    continue;
                }
                let chart = std::array::from_fn(|i| a.chart[i] / 2. + b.chart[i] / 2.);
                let world = surface
                    .point(chart)
                    .ok_or_else(|| fail(ErrorKind::Geometry, "unresolved curved interior point"))?;
                let chord = std::array::from_fn(|i| a.world[i] / 2. + b.world[i] / 2.);
                if world != chord {
                    candidates.push(ChartVertex {
                        chart,
                        world,
                        source: SampleKey::FacePoint {
                            face: face_id,
                            index: vertices.len() - boundary_count,
                        },
                    });
                } else if span > radius * crate::analytic_step::chord_angle(*radius, chord_mm) {
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
        let (loops, adjustments) = match self.surface(face_id) {
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
                (vec![trims.boundary], trims.adjustments)
            }
            None => return Err(fail(ErrorKind::Geometry, "unknown face")),
        };
        let face = &self.body().faces[&face_id];
        let mut vertices = Vec::new();
        for (bound, l) in face.bounds.iter().zip(&loops) {
            let mut samples = Vec::new();
            for usage in &bound.edges {
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
                }
            }
            if !bound.forward {
                samples.reverse();
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
        }
        let mut mesh = triangulate(
            face_id,
            &loops,
            vertices,
            max_work,
            self.surface(face_id),
            self.chord_tolerance_mm(),
        )?;
        mesh.chart_adjustments = adjustments;
        Ok(mesh)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
        let mesh = triangulate(0, &loops, v, 10000, None, 0.05).unwrap();
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
}
