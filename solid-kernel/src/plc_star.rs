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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KernelQuality {
    pub min_scaled_jacobian: f64,
    pub max_edge_ratio: f64,
}
#[derive(Debug, Clone)]
pub struct KernelTrial {
    pub before_point: V3,
    pub candidate_point: V3,
    pub accepted: bool,
    pub before_quality: KernelQuality,
    pub candidate_quality: Option<KernelQuality>,
    pub after_quality: KernelQuality,
    pub refusal: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialPhase {
    OriginalQuality,
    CandidatePosition,
    CandidateGeometry,
    CandidateQuality,
    Complete,
}
#[derive(Debug, Clone, Copy)]
pub struct TrialProgress {
    pub phase: TrialPhase,
    pub completed: usize,
    pub total: usize,
}
fn kernel_quality(
    topology: &TetTopology,
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<KernelQuality, Error> {
    let n = topology.cells().len();
    let mut result = KernelQuality {
        min_scaled_jacobian: f64::INFINITY,
        max_edge_ratio: 0.,
    };
    for (i, t) in topology.cells().iter().enumerate() {
        if i % 256 == 0 && !progress(i, n) {
            return Err(fail(ErrorKind::Cancelled, "kernel quality trial cancelled"));
        }
        let q = fea_core::meshing::quality::element_quality(
            fea_core::ElementKind::Tet4,
            &t.map(|v| topology.points()[v]),
        );
        if !q.scaled_jacobian.is_finite() || !q.edge_ratio.is_finite() {
            return Err(fail(ErrorKind::Geometry, "nonfinite kernel quality metric"));
        }
        result.min_scaled_jacobian = result.min_scaled_jacobian.min(q.scaled_jacobian);
        result.max_edge_ratio = result.max_edge_ratio.max(q.edge_ratio);
    }
    if !progress(n, n) {
        return Err(fail(ErrorKind::Cancelled, "kernel quality trial cancelled"));
    }
    Ok(result)
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
        let topology = Self::topology_for(&plc, center, max_cells, &mut progress)?;
        Ok(Self { plc, topology })
    }
    fn topology_for(
        plc: &ValidatedPlc,
        center: V3,
        max_cells: usize,
        progress: &mut dyn FnMut(usize, usize) -> bool,
    ) -> Result<TetTopology, Error> {
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
        Ok(topology)
    }
    /// Try one deterministic interior-point relocation, retaining every boundary
    /// point/facet. Accept only a strict minimum-Jacobian improvement with no
    /// increase in worst edge ratio. This is not general sliver elimination.
    pub fn try_vertex_mean_kernel(
        mut self,
        mut progress: impl FnMut(TrialProgress) -> bool,
    ) -> Result<(Self, KernelTrial), Error> {
        let before_point = self.topology.points()[self.plc.vertices().len()];
        let before_quality = kernel_quality(&self.topology, &mut |completed, total| {
            progress(TrialProgress {
                phase: TrialPhase::OriginalQuality,
                completed,
                total,
            })
        })?;
        let origin = self.plc.vertices()[0];
        let mut sum = [0.; 3];
        let mut correction = [0.; 3];
        let n = self.plc.vertices().len() as f64;
        for (i, p) in self.plc.vertices().iter().enumerate() {
            if i % 256 == 0
                && !progress(TrialProgress {
                    phase: TrialPhase::CandidatePosition,
                    completed: i,
                    total: self.plc.vertices().len(),
                })
            {
                return Err(fail(
                    ErrorKind::Cancelled,
                    "kernel position trial cancelled",
                ));
            }
            for axis in 0..3 {
                let term = (p[axis] - origin[axis]) / n - correction[axis];
                let next = sum[axis] + term;
                correction[axis] = (next - sum[axis]) - term;
                sum[axis] = next;
            }
        }
        let candidate_point = std::array::from_fn(|axis| origin[axis] + sum[axis]);
        let mut report = KernelTrial {
            before_point,
            candidate_point,
            accepted: false,
            before_quality,
            candidate_quality: None,
            after_quality: before_quality,
            refusal: None,
        };
        let mut candidate = None;
        if candidate_point == before_point {
            report.refusal = Some("candidate equals current kernel point".into());
        } else {
            match Self::topology_for(
                &self.plc,
                candidate_point,
                self.topology.cells().len(),
                &mut |completed, total| {
                    progress(TrialProgress {
                        phase: TrialPhase::CandidateGeometry,
                        completed,
                        total,
                    })
                },
            ) {
                Ok(topology) => {
                    let quality = kernel_quality(&topology, &mut |completed, total| {
                        progress(TrialProgress {
                            phase: TrialPhase::CandidateQuality,
                            completed,
                            total,
                        })
                    })?;
                    report.candidate_quality = Some(quality);
                    if quality.min_scaled_jacobian > before_quality.min_scaled_jacobian
                        && quality.max_edge_ratio <= before_quality.max_edge_ratio
                    {
                        report.accepted = true;
                        report.after_quality = quality;
                        candidate = Some(topology);
                    } else {
                        report.refusal =
                            Some("candidate does not improve minimum Jacobian without worsening edge ratio".into());
                    }
                }
                Err(e) if e.kind == ErrorKind::Geometry => {
                    report.refusal = Some(e.detail);
                }
                Err(e) => return Err(e),
            }
        }
        if !progress(TrialProgress {
            phase: TrialPhase::Complete,
            completed: 1,
            total: 1,
        }) {
            return Err(fail(ErrorKind::Cancelled, "kernel quality trial cancelled"));
        }
        if let Some(topology) = candidate {
            self.topology = topology;
        }
        Ok((self, report))
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
