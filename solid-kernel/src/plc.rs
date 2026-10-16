//! Immutable triangulated PLC admission from globally checked sampled geometry.
//! One closed connected shell only. This is input validation, not constrained
//! tetrahedralization, cavity/region support or exact CAD boundary certification.
use crate::{
    step::{EntityReference, Error, ErrorKind},
    triangle_intersection::{CheckedSurface, NativeDistanceSolid},
    V3,
};
use std::collections::BTreeSet;
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub facets: usize,
    pub segments: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            facets: 2_000_000,
            segments: 6_000_000,
        }
    }
}
pub struct ValidatedPlc {
    checked: CheckedSurface,
    segments: Vec<[usize; 2]>,
}
fn fail(kind: ErrorKind, detail: &str) -> Error {
    Error {
        offset: 0,
        kind,
        detail: detail.into(),
    }
}
impl ValidatedPlc {
    /// Reuse the immutable global facet proof. Every triangulated boundary edge
    /// is an explicit constraint, including diagonals inside a sampled CAD face.
    /// No tolerance welding, segment splitting, point deletion or hidden repair.
    pub fn new(
        checked: CheckedSurface,
        limits: Limits,
        mut progress: impl FnMut(usize, usize) -> bool,
    ) -> Result<Self, Error> {
        let facets = &checked.surface().mesh.triangles;
        if facets.len() > limits.facets {
            return Err(fail(ErrorKind::Limit, "PLC facet budget exceeded"));
        }
        if !progress(0, facets.len()) {
            return Err(fail(ErrorKind::Cancelled, "PLC admission cancelled"));
        }
        let mut segments = BTreeSet::new();
        for (i, tri) in facets.iter().enumerate() {
            if i % 256 == 0 && !progress(i, facets.len()) {
                return Err(fail(ErrorKind::Cancelled, "PLC admission cancelled"));
            }
            for j in 0..3 {
                let (a, b) = (tri[j], tri[(j + 1) % 3]);
                let edge = [a.min(b), a.max(b)];
                if !segments.contains(&edge) {
                    if segments.len() >= limits.segments {
                        return Err(fail(ErrorKind::Limit, "PLC segment budget exceeded"));
                    }
                    segments.insert(edge);
                }
            }
        }
        if !progress(facets.len(), facets.len()) {
            return Err(fail(ErrorKind::Cancelled, "PLC admission cancelled"));
        }
        Ok(Self {
            checked,
            segments: segments.into_iter().collect(),
        })
    }
    pub fn vertices(&self) -> &[V3] {
        &self.checked.surface().mesh.vertices
    }
    pub fn facets(&self) -> &[[usize; 3]] {
        &self.checked.surface().mesh.triangles
    }
    pub fn segments(&self) -> &[[usize; 2]] {
        &self.segments
    }
    pub fn facet_source_reference(&self, facet: usize) -> Option<EntityReference> {
        let surface = self.checked.surface();
        let tag = *surface.mesh.face.get(facet)?;
        Some(EntityReference {
            document: surface.source_document?,
            entity: *surface.source_faces.get(tag)?,
        })
    }
    /// Read-only source/report and sampled validation evidence. No mutable
    /// geometry escape exists while this PLC remains admitted.
    pub fn checked_surface(&self) -> &CheckedSurface {
        &self.checked
    }
    pub fn into_distance_solid(self) -> NativeDistanceSolid {
        self.checked.into_distance_solid()
    }
}
