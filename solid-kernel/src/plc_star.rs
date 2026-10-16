//! Boundary-conforming fan seed for strictly star-shaped triangulated PLCs.
//! Not constrained Delaunay, general PLC recovery or a production strategy.
use crate::{
    plc::ValidatedPlc,
    step::{Error, ErrorKind},
    V3,
};
use fea_core::meshing::tet_topology::{orientation, TetTopology};
use std::{cmp::Ordering, collections::BTreeSet};
fn fail(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        offset: 0,
        kind,
        detail: detail.into(),
    }
}
pub struct StarSeed {
    plc: ValidatedPlc,
    topology: TetTopology,
}
impl StarSeed {
    /// The caller supplies a kernel point. Every outward boundary facet must
    /// see it strictly on its interior side, certified by exact orientation.
    /// No epsilon, centroid classification, feature deletion or fallback.
    pub fn new(
        plc: ValidatedPlc,
        center: V3,
        max_cells: usize,
        mut progress: impl FnMut(usize, usize) -> bool,
    ) -> Result<Self, Error> {
        let n = plc.facets().len();
        if n > max_cells {
            return Err(fail(
                ErrorKind::Limit,
                "star seed tetrahedron budget exceeded",
            ));
        }
        if !progress(0, n) {
            return Err(fail(ErrorKind::Cancelled, "star seed cancelled"));
        }
        let mut cells = Vec::with_capacity(n);
        let center_id = plc.vertices().len();
        for (i, tri) in plc.facets().iter().enumerate() {
            if i % 256 == 0 && !progress(i, n) {
                return Err(fail(ErrorKind::Cancelled, "star seed cancelled"));
            }
            let [a, b, c] = tri.map(|id| plc.vertices()[id]);
            let side = orientation(a, b, c, center)
                .map_err(|e| fail(ErrorKind::Geometry, e.to_string()))?;
            if side != Ordering::Less {
                return Err(fail(ErrorKind::Geometry,format!("kernel point is not strictly inside boundary facet {i}; star seed unsupported")));
            }
            cells.push([center_id, tri[0], tri[1], tri[2]]);
        }
        let mut points = plc.vertices().to_vec();
        points.push(center);
        let topology = TetTopology::new(points, cells, max_cells)
            .map_err(|e| fail(ErrorKind::Topology, e.to_string()))?;
        let keys = |faces: &[[usize; 3]]| -> BTreeSet<[usize; 3]> {
            faces
                .iter()
                .map(|f| {
                    let mut f = *f;
                    f.sort_unstable();
                    f
                })
                .collect()
        };
        if keys(topology.boundary()) != keys(plc.facets()) {
            return Err(fail(
                ErrorKind::Topology,
                "star seed did not preserve every PLC facet",
            ));
        }
        if !progress(n, n) {
            return Err(fail(ErrorKind::Cancelled, "star seed cancelled"));
        }
        Ok(Self { plc, topology })
    }
    /// Convert only through existing quality/conformity gates, then restore
    /// source-tagged boundary regions from the exactly preserved PLC facets.
    pub fn into_solver_mesh(
        self,
        material: fea_core::Elastic,
    ) -> Result<crate::native_volume::NativeBoundaryMesh, fea_core::meshing::MeshError> {
        if self
            .plc
            .checked_surface()
            .surface()
            .source_document
            .is_none()
        {
            return Err(fea_core::meshing::MeshError::Unsupported(
                "source-tagged solver conversion requires a known STEP document".into(),
            ));
        }
        let mut mesh = self.topology.to_mesh(material)?;
        let (surface, metadata) = self.plc.into_checked_surface().into_geometry_parts();
        mesh.surfaces.clear();
        for (facet, tag) in surface.triangles.into_iter().zip(surface.face) {
            mesh.surfaces
                .entry(format!("face{}", tag + 1))
                .or_default()
                .push(facet.to_vec());
        }
        Ok(crate::native_volume::NativeBoundaryMesh::new(
            mesh, metadata,
        ))
    }
    pub fn topology(&self) -> &TetTopology {
        &self.topology
    }
    pub fn plc(&self) -> &ValidatedPlc {
        &self.plc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_source_cannot_create_source_tagged_solver_regions() {
        let surface = crate::native_surface::SampledSurface {
            source_document: None,
            face_orientation_adjustments: vec![],
            chart_adjustments: vec![],
            source_faces: vec![1, 2, 3, 4],
            signed_volume_mm3: 1. / 6.,
            mesh: crate::SurfaceMesh {
                vertices: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                triangles: vec![[1, 2, 3], [0, 3, 2], [0, 1, 3], [0, 2, 1]],
                face: vec![0, 1, 2, 3],
            },
        };
        let checked = surface.check_intersections(100, |_| true).unwrap();
        let plc = ValidatedPlc::new(checked, crate::plc::Limits::default(), |_, _| true).unwrap();
        let seed = StarSeed::new(plc, [0.25; 3], 100, |_, _| true).unwrap();
        let material = fea_core::Elastic::new(200e3, 0.3);
        seed.topology().to_mesh(material).unwrap();
        assert!(matches!(
            seed.into_solver_mesh(material),
            Err(fea_core::meshing::MeshError::Unsupported(_))
        ));
    }
}
