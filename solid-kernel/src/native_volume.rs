//! Diagnostic association-preserving bridge to the existing lattice volume
//! mesher. Not constrained PLC recovery or exact CAD boundary certification.
use crate::{
    step::{EntityReference, Error, ErrorKind},
    triangle_intersection::{NativeDistanceSolid, NativeGeometryMetadata},
    VolumeParams, V3,
};
use fea_core::{mesh::Mesh, tetmesh::MeshReport, Elastic};

/// Geometry and source associations stay together while selections are resolved.
/// Any caller modifying the mesh after `into_parts` must revalidate associations.
#[derive(Debug)]
pub struct NativeVolumeMesh {
    mesh: Mesh,
    report: MeshReport,
    metadata: NativeGeometryMetadata,
}
impl NativeVolumeMesh {
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }
    pub fn report(&self) -> &MeshReport {
        &self.report
    }
    pub fn metadata(&self) -> &NativeGeometryMetadata {
        &self.metadata
    }
    /// Return existing generated facets for a stored CAD face selection. Empty
    /// or missing regions are refused; this does not certify complete coverage.
    pub fn boundary(&self, reference: EntityReference) -> Result<&[Vec<usize>], Error> {
        let tag = self.metadata.face_tag(reference)?;
        self.mesh
            .surfaces
            .get(&format!("face{}", tag + 1))
            .filter(|facets| !facets.is_empty())
            .map(Vec::as_slice)
            .ok_or_else(|| Error {
                offset: 0,
                kind: ErrorKind::Geometry,
                detail: format!(
                    "source face #{} is unresolved in the generated volume mesh",
                    reference.entity
                ),
            })
    }
    /// Explicitly retain unresolved engineering faces; never rebind them to a
    /// neighbouring region or silently delete their stored selection.
    pub fn unresolved_faces(&self) -> Vec<EntityReference> {
        (0..self.metadata.source_faces.len())
            .filter_map(|tag| self.metadata.face_reference(tag))
            .filter(|reference| self.boundary(*reference).is_err())
            .collect()
    }
    pub fn into_parts(self) -> (Mesh, MeshReport, NativeGeometryMetadata) {
        (self.mesh, self.report, self.metadata)
    }
}
impl NativeDistanceSolid {
    /// Existing sampled-distance lattice meshing, retaining source metadata.
    /// Coordinates are mm; the unit-free material must use consistent units
    /// (e.g. N/mm² for Young's modulus). No material conversion is inferred.
    pub fn mesh_volume(
        &self,
        material: Elastic,
        params: VolumeParams,
        size: &dyn Fn(V3) -> f64,
    ) -> Result<NativeVolumeMesh, String> {
        if self.metadata().source_document.is_none() {
            return Err("native volume associations require a known STEP source document".into());
        }
        let (mesh, report) = crate::mesh_volume(self.solid(), material, params, size)?;
        Ok(NativeVolumeMesh {
            mesh,
            report,
            metadata: self.metadata().clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step::DocumentFingerprint;
    #[test]
    fn missing_volume_regions_never_rebind_stored_cad_selections() {
        let document = DocumentFingerprint([7; 32]);
        let metadata = NativeGeometryMetadata {
            source_document: Some(document),
            source_faces: vec![99, 123, 456],
            chart_adjustments: vec![],
            face_orientation_adjustments: vec![],
        };
        let mut mesh = Mesh::new(fea_core::Physics::Solid);
        mesh.surfaces.insert("face1".into(), vec![vec![0, 1, 2]]);
        mesh.surfaces.insert("face2".into(), vec![]);
        let generated = NativeVolumeMesh {
            mesh,
            metadata,
            report: MeshReport {
                tets: 0,
                nodes: 0,
                volume: 0.,
                worst_quality: 0.,
                snapped: 0,
                refused: 0,
            },
        };
        let references: Vec<_> = (0..3)
            .map(|tag| generated.metadata().face_reference(tag).unwrap())
            .collect();
        assert_eq!(generated.boundary(references[0]).unwrap(), &[vec![0, 1, 2]]);
        assert!(generated
            .boundary(references[1])
            .unwrap_err()
            .detail
            .contains("unresolved"));
        assert!(generated.boundary(references[2]).is_err());
        assert_eq!(generated.unresolved_faces(), references[1..]);
        assert!(generated
            .boundary(EntityReference {
                document: DocumentFingerprint([8; 32]),
                ..references[0]
            })
            .is_err());
        let (_, _, metadata) = generated.into_parts();
        assert_eq!(metadata.source_document, Some(document));
    }
}
