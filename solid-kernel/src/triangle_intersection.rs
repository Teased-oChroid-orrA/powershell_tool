//! Bounded exact-sign intersection checks for the sampled triangle surface.
//! This verifies facets, not the original exact trimmed CAD geometry.
use crate::{
    cross,
    native_surface::SampledSurface,
    planar_step::{intersects, sign},
    step::{Error, ErrorKind},
    sub, SurfaceMesh, V3,
};
use fea_core::meshing::tet_topology::orientation;
use std::cmp::Ordering;
fn fail(kind: ErrorKind, text: impl Into<String>) -> Error {
    Error {
        offset: 0,
        kind,
        detail: text.into(),
    }
}
fn orient(a: V3, b: V3, c: V3, d: V3) -> Result<Ordering, Error> {
    orientation(a, b, c, d).map_err(|_| {
        fail(
            ErrorKind::Geometry,
            "surface intersection predicate arithmetic range exceeded",
        )
    })
}
fn point_in_triangle(p: [f64; 2], t: [[f64; 2]; 3]) -> Result<bool, Error> {
    let s = [
        sign(t[0], t[1], p)?,
        sign(t[1], t[2], p)?,
        sign(t[2], t[0], p)?,
    ];
    Ok(s.iter().all(|s| *s != Ordering::Less) || s.iter().all(|s| *s != Ordering::Greater))
}
fn coplanar_segment(
    mesh: &SurfaceMesh,
    edge: [usize; 2],
    triangle: [usize; 3],
) -> Result<bool, Error> {
    let t = triangle.map(|i| mesh.vertices[i]);
    let normal = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let axis = (0..3)
        .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
        .unwrap();
    if normal[axis] == 0. {
        return Err(fail(
            ErrorKind::Geometry,
            "degenerate intersection triangle",
        ));
    }
    let project = |p: V3| [p[(axis + 1) % 3], p[(axis + 2) % 3]];
    let tri = t.map(project);
    let mut edge = edge;
    if triangle.contains(&edge[1]) {
        edge.swap(0, 1);
    }
    let [a, b] = edge.map(|i| project(mesh.vertices[i]));
    let shared = triangle.contains(&edge[0]);
    if point_in_triangle(b, tri)? || (!shared && point_in_triangle(a, tri)?) {
        return Ok(true);
    }
    for i in 0..3 {
        let c = tri[i];
        let d = tri[(i + 1) % 3];
        if shared && (triangle[i] == edge[0] || triangle[(i + 1) % 3] == edge[0]) {
            let other = if triangle[i] == edge[0] { d } else { c };
            if sign(a, b, other)? == Ordering::Equal {
                // Collinear rays from the permitted common endpoint may overlap.
                let axis = if b[0] != a[0] { 0 } else { 1 };
                if (b[axis] > a[axis]) == (other[axis] > a[axis]) {
                    return Ok(true);
                }
            }
        } else if intersects(a, b, c, d)? {
            return Ok(true);
        }
    }
    Ok(false)
}
fn segment_triangle(
    mesh: &SurfaceMesh,
    edge: [usize; 2],
    triangle: [usize; 3],
) -> Result<bool, Error> {
    if edge.iter().all(|i| triangle.contains(i)) {
        return Ok(false);
    }
    let [p, q] = edge.map(|i| mesh.vertices[i]);
    let [a, b, c] = triangle.map(|i| mesh.vertices[i]);
    let s = orient(a, b, c, p)?;
    let t = orient(a, b, c, q)?;
    if s == t && s != Ordering::Equal {
        return Ok(false);
    }
    if s == Ordering::Equal && t == Ordering::Equal {
        return coplanar_segment(mesh, edge, triangle);
    }
    // A noncoplanar edge can meet the plane at its permitted shared endpoint
    // only once. Coplanar overlap is handled separately above.
    if (s == Ordering::Equal && triangle.contains(&edge[0]))
        || (t == Ordering::Equal && triangle.contains(&edge[1]))
    {
        return Ok(false);
    }
    let side = [
        orient(p, q, a, b)?,
        orient(p, q, b, c)?,
        orient(p, q, c, a)?,
    ];
    Ok(side.iter().all(|s| *s != Ordering::Less) || side.iter().all(|s| *s != Ordering::Greater))
}
fn triangles_intersect(mesh: &SurfaceMesh, a: [usize; 3], b: [usize; 3]) -> Result<bool, Error> {
    for i in 0..3 {
        if segment_triangle(mesh, [a[i], a[(i + 1) % 3]], b)?
            || segment_triangle(mesh, [b[i], b[(i + 1) % 3]], a)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}
/// Immutable ownership of a closed, consistently oriented, nonintersecting
/// sampled facet set. This is not certification of the original exact CAD.
#[derive(Debug)]
pub struct CheckedSurface {
    inner: SampledSurface,
    pairs_checked: usize,
}
/// Source association and every explicitly requested repair, retained together.
#[derive(Debug)]
pub struct NativeGeometryMetadata {
    pub source_document: Option<crate::step::DocumentFingerprint>,
    pub source_faces: Vec<u64>,
    pub chart_adjustments: Vec<crate::cylindrical_step::ChartAdjustment>,
    pub face_orientation_adjustments: Vec<crate::orientation_repair::FaceOrientationChange>,
}
impl NativeGeometryMetadata {
    /// Construct a document-scoped face reference; no positional/tag fallback.
    pub fn face_reference(&self, tag: usize) -> Option<crate::step::EntityReference> {
        Some(crate::step::EntityReference {
            document: self.source_document?,
            entity: *self.source_faces.get(tag)?,
        })
    }
}
/// Checked single-shell distance geometry. Assemblies and nested/cavity shells
/// are not admitted by this native gate.
pub struct NativeDistanceSolid {
    solid: crate::SurfaceSolid,
    metadata: NativeGeometryMetadata,
}
impl NativeDistanceSolid {
    pub fn solid(&self) -> &crate::SurfaceSolid {
        &self.solid
    }
    pub fn source_faces(&self) -> &[u64] {
        &self.metadata.source_faces
    }
    pub fn chart_adjustments(&self) -> &[crate::cylindrical_step::ChartAdjustment] {
        &self.metadata.chart_adjustments
    }
    pub fn face_orientation_adjustments(
        &self,
    ) -> &[crate::orientation_repair::FaceOrientationChange] {
        &self.metadata.face_orientation_adjustments
    }
    pub fn into_parts(self) -> (crate::SurfaceSolid, NativeGeometryMetadata) {
        (self.solid, self.metadata)
    }
}
impl CheckedSurface {
    pub fn into_distance_solid(self) -> NativeDistanceSolid {
        NativeDistanceSolid {
            solid: crate::SurfaceSolid::from_oriented_mesh(self.inner.mesh),
            metadata: NativeGeometryMetadata {
                source_document: self.inner.source_document,
                source_faces: self.inner.source_faces,
                chart_adjustments: self.inner.chart_adjustments,
                face_orientation_adjustments: self.inner.face_orientation_adjustments,
            },
        }
    }
    pub fn surface(&self) -> &SampledSurface {
        &self.inner
    }
    pub fn pairs_checked(&self) -> usize {
        self.pairs_checked
    }
}
impl SampledSurface {
    /// Consume the sampled surface so its mesh cannot be mutated behind a
    /// successful check. AABB broad phase and exact-sign segment/triangle tests
    /// permit only shared topological vertices/edges. Callback false cancels.
    pub fn check_intersections(
        mut self,
        max_pairs: usize,
        mut progress: impl FnMut(usize) -> bool,
    ) -> Result<CheckedSurface, Error> {
        if !progress(0) {
            return Err(fail(
                ErrorKind::Cancelled,
                "surface intersection check cancelled",
            ));
        }
        let mesh = &self.mesh;
        if mesh.triangles.is_empty() || mesh.triangles.len() > max_pairs {
            return Err(fail(
                ErrorKind::Limit,
                "surface intersection admission budget exceeded",
            ));
        }
        if mesh.vertices.len() > mesh.triangles.len().saturating_mul(3)
            || mesh.face.iter().any(|f| *f >= self.source_faces.len())
            || mesh.face.len() != mesh.triangles.len()
            || mesh
                .vertices
                .iter()
                .flatten()
                .any(|x| !x.is_finite() || (*x != 0. && !(1e-30..=1e30).contains(&x.abs())))
            || mesh
                .triangles
                .iter()
                .flatten()
                .any(|i| *i >= mesh.vertices.len())
        {
            return Err(fail(
                ErrorKind::Geometry,
                "invalid surface intersection input",
            ));
        }
        crate::native_surface::validate_connectivity(mesh)?;
        let volume = crate::native_surface::signed_volume(mesh)?;
        if !volume.is_finite() || volume <= 0. {
            return Err(fail(
                ErrorKind::Geometry,
                "invalid sampled surface signed volume",
            ));
        }
        let mut bounds = Vec::new();
        for (i, t) in mesh.triangles.iter().enumerate() {
            let points = t.map(|v| mesh.vertices[v]);
            if cross(sub(points[1], points[0]), sub(points[2], points[0]))
                .iter()
                .all(|x| *x == 0.)
            {
                return Err(fail(ErrorKind::Geometry, "degenerate sampled facet"));
            }
            let lo = std::array::from_fn::<_, 3, _>(|j| {
                points.iter().map(|p| p[j]).fold(f64::INFINITY, f64::min)
            });
            let hi = std::array::from_fn::<_, 3, _>(|j| {
                points
                    .iter()
                    .map(|p| p[j])
                    .fold(f64::NEG_INFINITY, f64::max)
            });
            bounds.push((i, lo, hi));
        }
        bounds.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]).then(a.0.cmp(&b.0)));
        let mut checks = 0;
        for (i, &(a, lo, hi)) in bounds.iter().enumerate() {
            for &(b, other_lo, other_hi) in bounds.iter().skip(i + 1) {
                if other_lo[0] > hi[0] {
                    break;
                }
                if checks >= max_pairs {
                    return Err(fail(
                        ErrorKind::Limit,
                        "surface intersection pair budget exceeded",
                    ));
                }
                checks += 1;
                if checks % 256 == 0 && !progress(checks) {
                    return Err(fail(
                        ErrorKind::Cancelled,
                        "surface intersection check cancelled",
                    ));
                }
                if (1..3).any(|axis| other_lo[axis] > hi[axis] || other_hi[axis] < lo[axis]) {
                    continue;
                }
                if triangles_intersect(mesh, mesh.triangles[a], mesh.triangles[b])? {
                    return Err(fail(
                        ErrorKind::Geometry,
                        format!("sampled triangles {a} and {b} intersect beyond shared topology"),
                    ));
                }
            }
        }
        if !progress(checks) {
            return Err(fail(
                ErrorKind::Cancelled,
                "surface intersection check cancelled",
            ));
        }
        self.signed_volume_mm3 = volume;
        Ok(CheckedSurface {
            inner: self,
            pairs_checked: checks,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn mesh(points: Vec<V3>) -> SurfaceMesh {
        SurfaceMesh {
            vertices: points,
            ..SurfaceMesh::default()
        }
    }
    #[test]
    fn exact_triangle_checks_distinguish_allowed_and_invalid_contacts() {
        let m = mesh(vec![
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [1., 1., -1.],
            [1., 1., 1.],
            [2., 2., 0.],
            [-1., 0., 0.],
            [0., -1., 0.],
        ]);
        assert!(triangles_intersect(&m, [0, 1, 2], [3, 4, 5]).unwrap());
        assert!(!triangles_intersect(&m, [0, 1, 2], [0, 6, 7]).unwrap());
        assert!(!triangles_intersect(&m, [0, 1, 2], [1, 5, 2]).unwrap());
        assert!(triangles_intersect(&m, [0, 1, 2], [0, 1, 5]).unwrap());
        let coincident = mesh(vec![
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
        ]);
        assert!(triangles_intersect(&coincident, [0, 1, 2], [3, 4, 5]).unwrap());
    }
}

#[cfg(test)]
mod checked_surface_tests {
    use super::*;
    fn tetra_pair(shift: f64) -> SampledSurface {
        let mut mesh = SurfaceMesh::default();
        for offset in [0., shift] {
            let base = mesh.vertices.len();
            mesh.vertices.extend([
                [offset, 0., 0.],
                [offset + 1., 0., 0.],
                [offset, 1., 0.],
                [offset, 0., 1.],
            ]);
            for t in [[1, 2, 3], [0, 3, 2], [0, 1, 3], [0, 2, 1]] {
                mesh.triangles.push(t.map(|i| base + i));
                mesh.face.push(0);
            }
        }
        SampledSurface {
            source_document: None,
            face_orientation_adjustments: Vec::new(),
            chart_adjustments: Vec::new(),
            mesh,
            source_faces: vec![42],
            signed_volume_mm3: 99.,
        }
    }
    #[test]
    fn crossing_closed_shell_is_refused_and_embedded_shell_passes() {
        let mut mesh = SurfaceMesh {
            vertices: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [1., 1., 0.],
                [0., 1., 0.],
                [0., 0., 1.],
                [1., 0., 1.],
                [0.5, 0.5, -0.5],
                [0., 1., 1.],
            ],
            ..SurfaceMesh::default()
        };
        mesh.triangles = vec![
            [0, 3, 2],
            [0, 2, 1],
            [4, 5, 6],
            [4, 6, 7],
            [0, 4, 7],
            [0, 7, 3],
            [1, 2, 6],
            [1, 6, 5],
            [0, 1, 5],
            [0, 5, 4],
            [3, 7, 6],
            [3, 6, 2],
        ];
        mesh.face = vec![0; 12];
        let folded = SampledSurface {
            source_document: None,
            face_orientation_adjustments: Vec::new(),
            chart_adjustments: Vec::new(),
            mesh,
            source_faces: vec![42],
            signed_volume_mm3: 99.,
        };
        assert!(folded
            .check_intersections(1000, |_| true)
            .unwrap_err()
            .detail
            .contains("intersect beyond shared topology"));
        let mut tetra = tetra_pair(3.);
        tetra.mesh.vertices.truncate(4);
        tetra.mesh.triangles.truncate(4);
        tetra.mesh.face.truncate(4);
        let checked = tetra.check_intersections(1000, |_| true).unwrap();
        assert!((checked.surface().signed_volume_mm3 - 1. / 6.).abs() < 1e-14);
        let native = checked.into_distance_solid();
        assert!((native.solid().volume() - 1. / 6.).abs() < 1e-14);
        assert_eq!(native.source_faces(), &[42]);
        assert!(tetra_pair(3.)
            .check_intersections(1000, |_| true)
            .unwrap_err()
            .detail
            .contains("one connected shell"));
    }
    #[test]
    fn admission_cancellation_and_mutated_unchecked_input_cannot_create_a_proof() {
        assert_eq!(
            tetra_pair(3.)
                .check_intersections(0, |_| true)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(
            tetra_pair(3.)
                .check_intersections(1000, |_| false)
                .unwrap_err()
                .kind,
            ErrorKind::Cancelled
        );
        let mut invalid = tetra_pair(3.);
        invalid.mesh.vertices[0][0] = f64::NAN;
        assert_eq!(
            invalid
                .check_intersections(1000, |_| true)
                .unwrap_err()
                .kind,
            ErrorKind::Geometry
        );
        let mut invalid = tetra_pair(3.);
        invalid.mesh.triangles[0][0] = usize::MAX;
        assert_eq!(
            invalid
                .check_intersections(1000, |_| true)
                .unwrap_err()
                .kind,
            ErrorKind::Geometry
        );
        let mut invalid = tetra_pair(3.);
        invalid.mesh.face[0] = usize::MAX;
        assert_eq!(
            invalid
                .check_intersections(1000, |_| true)
                .unwrap_err()
                .kind,
            ErrorKind::Geometry
        );
    }
}
