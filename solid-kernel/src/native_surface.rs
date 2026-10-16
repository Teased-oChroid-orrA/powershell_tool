//! Diagnostic sampled surface assembly with exact source-boundary identity.
//! Closure checks do not certify global nonintersection or exact CAD validity.
use crate::{
    analytic_step::AnalyticModel,
    chart_mesh::SampleKey,
    cross, dot,
    step::{Error, ErrorKind},
    sub, SurfaceMesh,
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub orientation_policy: crate::orientation_repair::OrientationPolicy,
    pub chart_policy: crate::cylindrical_step::ChartPolicy,
    pub vertices: usize,
    pub triangles: usize,
    pub face_work: usize,
    pub total_work: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            orientation_policy: crate::orientation_repair::OrientationPolicy::Strict,
            chart_policy: crate::cylindrical_step::ChartPolicy::Strict,
            vertices: 1_000_000,
            triangles: 2_000_000,
            face_work: 2_000_000,
            total_work: 80_000_000,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationPhase {
    Triangulating,
    RepairingOrientation,
    Complete,
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub phase: GenerationPhase,
    pub completed_work: usize,
    pub total_work: usize,
    pub completed_faces: usize,
    pub total_faces: usize,
}
#[derive(Debug)]
pub struct SampledSurface {
    pub source_document: Option<crate::step::DocumentFingerprint>,
    pub face_orientation_adjustments: Vec<crate::orientation_repair::FaceOrientationChange>,
    pub chart_adjustments: Vec<crate::cylindrical_step::ChartAdjustment>,
    pub mesh: SurfaceMesh,
    /// Face tags map to source IDs only within this physical file. No persistent
    /// naming across CAD edits/re-exports is promised.
    pub source_faces: Vec<u64>,
    pub signed_volume_mm3: f64,
}
fn fail(kind: ErrorKind, s: impl Into<String>) -> Error {
    Error {
        offset: 0,
        kind,
        detail: s.into(),
    }
}
pub(crate) fn validate_connectivity(mesh: &SurfaceMesh) -> Result<(), Error> {
    let mut edges = BTreeMap::<(usize, usize), (usize, i64)>::new();
    let mut unique = BTreeMap::new();
    for (triangle_id, t) in mesh.triangles.iter().enumerate() {
        let mut key = *t;
        key.sort();
        if let Some(previous) = unique.insert(key, triangle_id) {
            return Err(fail(
                ErrorKind::Topology,
                format!(
                    "duplicate assembled surface triangle: tags {} and {}; vertex IDs {:?}",
                    mesh.face[previous], mesh.face[triangle_id], key
                ),
            ));
        }
        for i in 0..3 {
            let a = t[i];
            let b = t[(i + 1) % 3];
            let entry = edges.entry((a.min(b), a.max(b))).or_default();
            entry.0 += 1;
            entry.1 += if a < b { 1 } else { -1 };
        }
    }
    if edges.values().any(|&(n, s)| n != 2 || s != 0) {
        return Err(fail(
            ErrorKind::Topology,
            "native sampled surface is not closed and consistently oriented",
        ));
    }
    // Edge incidence alone accepts a pinched vertex with several disjoint
    // fans. A closed 2-manifold requires one cyclic triangle link per vertex.
    let mut links = vec![Vec::new(); mesh.vertices.len()];
    for t in &mesh.triangles {
        for i in 0..3 {
            links[t[i]].push((t[(i + 1) % 3], t[(i + 2) % 3]));
        }
    }
    for link in links {
        let mut adjacency = BTreeMap::<usize, Vec<usize>>::new();
        for (a, b) in link {
            adjacency.entry(a).or_default().push(b);
            adjacency.entry(b).or_default().push(a);
        }
        if adjacency.values().any(|n| n.len() != 2) || adjacency.is_empty() {
            return Err(fail(ErrorKind::Topology, "invalid sampled vertex link"));
        }
        let mut reached = BTreeSet::new();
        let mut pending = vec![*adjacency.keys().next().unwrap()];
        while let Some(v) = pending.pop() {
            if reached.insert(v) {
                pending.extend(&adjacency[&v]);
            }
        }
        if reached.len() != adjacency.len() {
            return Err(fail(
                ErrorKind::Topology,
                "sampled vertex has disconnected triangle fans",
            ));
        }
    }
    let mut neighbours = vec![Vec::new(); mesh.vertices.len()];
    for &(a, b) in edges.keys() {
        neighbours[a].push(b);
        neighbours[b].push(a);
    }
    let mut reached = BTreeSet::new();
    let mut pending = vec![0];
    while let Some(v) = pending.pop() {
        if reached.insert(v) {
            pending.extend(&neighbours[v]);
        }
    }
    if reached.len() != mesh.vertices.len() {
        return Err(fail(
            ErrorKind::Topology,
            "native sampled surface requires one connected shell",
        ));
    }
    Ok(())
}
pub(crate) fn signed_volume(mesh: &SurfaceMesh) -> Result<f64, Error> {
    signed_volume_of(&mesh.vertices, mesh.triangles.iter().copied())
}
pub(crate) fn signed_volume_of(
    vertices: &[crate::V3],
    triangles: impl Iterator<Item = [usize; 3]>,
) -> Result<f64, Error> {
    let origin = vertices
        .first()
        .copied()
        .ok_or_else(|| fail(ErrorKind::Geometry, "empty sampled surface"))?;
    let mut volume = 0.;
    let mut correction = 0.;
    for t in triangles {
        let [a, b, c] = t.map(|i| sub(vertices[i], origin));
        let term = dot(a, cross(b, c)) / 6. - correction;
        let sum = volume + term;
        correction = (sum - volume) - term;
        volume = sum;
    }
    Ok(volume)
}
impl AnalyticModel {
    /// Assemble all faces or fail. Callback runs before each bounded face job and
    /// once at completion; false cancels without returning a partial surface.
    pub fn sampled_surface(
        &self,
        limits: Limits,
        mut progress: impl FnMut(Progress) -> bool,
    ) -> Result<SampledSurface, Error> {
        let total = self.body().faces.len();
        if limits.face_work == 0
            || total
                .checked_mul(limits.face_work)
                .is_none_or(|work| work > limits.total_work)
        {
            return Err(fail(ErrorKind::Limit, "surface chart work budget exceeded"));
        }
        let boundary_vertices = self
            .body()
            .edges
            .keys()
            .try_fold(self.body().vertices.len(), |n, id| {
                n.checked_add(self.edge_points(*id).unwrap().len() - 2)
            })
            .ok_or_else(|| fail(ErrorKind::Limit, "surface boundary count overflow"))?;
        if boundary_vertices > limits.vertices {
            return Err(fail(
                ErrorKind::Limit,
                "surface boundary exceeds the vertex budget",
            ));
        }
        let mut mesh = SurfaceMesh::default();
        let mut identities = BTreeMap::<SampleKey, usize>::new();
        let mut source_faces = Vec::new();
        let mut chart_adjustments = Vec::new();
        for (tag, face) in self.body().faces.values().enumerate() {
            if !progress(Progress {
                phase: GenerationPhase::Triangulating,
                completed_work: tag,
                total_work: total,
                completed_faces: tag,
                total_faces: total,
            }) {
                return Err(fail(
                    ErrorKind::Cancelled,
                    "native surface generation cancelled",
                ));
            }
            let chart = self.triangulate_sampled_chart_with_policy(
                face.id,
                limits.face_work,
                limits.chart_policy,
            )?;
            if chart.triangles.len() > limits.triangles.saturating_sub(mesh.triangles.len()) {
                return Err(fail(ErrorKind::Limit, "surface triangle budget exceeded"));
            }
            let mut map = Vec::new();
            for vertex in &chart.vertices {
                let id = if let Some(&id) = identities.get(&vertex.source) {
                    if mesh.vertices[id] != vertex.world {
                        return Err(fail(
                            ErrorKind::Geometry,
                            "shared boundary coordinate mismatch",
                        ));
                    }
                    id
                } else {
                    if mesh.vertices.len() >= limits.vertices {
                        return Err(fail(ErrorKind::Limit, "surface vertex budget exceeded"));
                    }
                    let id = mesh.vertices.len();
                    mesh.vertices.push(vertex.world);
                    identities.insert(vertex.source, id);
                    id
                };
                map.push(id);
            }
            for tri in chart.triangles {
                let mut t = tri.map(|i| map[i]);
                if t[0] == t[1] || t[1] == t[2] || t[2] == t[0] {
                    return Err(fail(
                        ErrorKind::Geometry,
                        format!(
                            "face #{} chart triangle collapses across a periodic seam",
                            face.id
                        ),
                    ));
                }
                let [a, b, c] = t.map(|i| mesh.vertices[i]);
                let center = std::array::from_fn(|i| {
                    tri.iter().map(|v| chart.vertices[*v].chart[i] / 3.).sum()
                });
                let normal = self.declared_face_normal(face.id, center).unwrap();
                let orientation = dot(cross(sub(b, a), sub(c, a)), normal);
                if !orientation.is_finite()
                    || orientation == 0.
                    || (orientation > 0.) != face.same_sense
                {
                    return Err(fail(
                        ErrorKind::Geometry,
                        "degenerate or locally inverted sampled surface triangle",
                    ));
                }
                // CDT faces are counterclockwise in the chart. Both supported
                // parameterizations have an outward geometric chart normal;
                // apply the source face sense once, never flip isolated facets.
                if !face.same_sense {
                    t.swap(1, 2);
                }
                mesh.triangles.push(t);
                mesh.face.push(tag);
            }
            chart_adjustments.extend(chart.chart_adjustments);
            source_faces.push(face.id);
        }
        let face_orientation_adjustments = match limits.orientation_policy {
            crate::orientation_repair::OrientationPolicy::Strict => Vec::new(),
            crate::orientation_repair::OrientationPolicy::RepairWithReport => {
                let senses = self
                    .body()
                    .faces
                    .values()
                    .map(|f| f.same_sense)
                    .collect::<Vec<_>>();
                crate::orientation_repair::repair(
                    &mut mesh,
                    &source_faces,
                    &senses,
                    |done, work| {
                        progress(Progress {
                            phase: GenerationPhase::RepairingOrientation,
                            completed_work: done,
                            total_work: work,
                            completed_faces: total,
                            total_faces: total,
                        })
                    },
                )?
            }
        };
        validate_connectivity(&mesh)?;
        let volume = signed_volume(&mesh)?;
        if !volume.is_finite() || volume <= 0. {
            return Err(fail(
                ErrorKind::Geometry,
                "nonpositive or nonfinite sampled shell volume",
            ));
        }
        if !progress(Progress {
            phase: GenerationPhase::Complete,
            completed_work: total,
            total_work: total,
            completed_faces: total,
            total_faces: total,
        }) {
            return Err(fail(
                ErrorKind::Cancelled,
                "native surface generation cancelled",
            ));
        }
        Ok(SampledSurface {
            source_document: Some(self.document_fingerprint()),
            face_orientation_adjustments,
            chart_adjustments,
            mesh,
            source_faces,
            signed_volume_mm3: volume,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_edges_do_not_hide_a_pinched_vertex() {
        let mut mesh = SurfaceMesh {
            vertices: vec![[0.; 3]; 7],
            ..SurfaceMesh::default()
        };
        for [a, b, c, d] in [[0, 1, 2, 3], [0, 4, 5, 6]] {
            mesh.triangles
                .extend([[b, c, d], [a, d, c], [a, b, d], [a, c, b]]);
        }
        let error = validate_connectivity(&mesh).unwrap_err();
        assert!(error.detail.contains("disconnected triangle fans"));
    }
}
