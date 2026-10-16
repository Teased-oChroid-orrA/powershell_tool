//! Backend-neutral CAD contracts. Kernel objects and IDs never cross this boundary.
//! Capability support refers to exposed operations, not acceptance of every exchange entity.
use crate::V3;
#[cfg(feature = "occt")]
use crate::{SurfaceMesh, SurfaceSolid};

/// One closed loop of a sketch profile, in the plane of the profile.
#[derive(Clone, Debug, PartialEq)]
pub enum Loop {
    Polygon(Vec<[f64; 2]>),
    Circle { centre: [f64; 2], radius: f64 },
}

/// An outer loop and the holes cut out of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub outer: Loop,
    pub holes: Vec<Loop>,
}

/// A face of a solid, in the order the kernel lists them (`SurfaceMesh::face` tags refer to this index). Normals are
/// not asked of the kernel (its normal query throws for a curved face, which aborts the process): use
/// `SurfaceSolid::face_normals`, taken from the tessellation.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceInfo {
    pub index: usize,
    pub area: f64,
    pub centre: V3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bool {
    Union,
    Cut,
    Intersect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendId {
    OpenCascade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Step,
    Iges,
    Primitives,
    Extrude,
    Boolean,
    Fillet,
    TaggedTessellation,
    Healing,
    PersistentTopology,
}

/// Compiled availability is distinct from implemented operation support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub backend: BackendId,
    pub available: bool,
}
impl Capabilities {
    pub fn supports(self, op: Operation) -> bool {
        self.available
            && matches!(
                op,
                Operation::Step
                    | Operation::Primitives
                    | Operation::Extrude
                    | Operation::Boolean
                    | Operation::Fillet
                    | Operation::TaggedTessellation
            )
    }
    pub fn require(self, op: Operation) -> Result<(), Unsupported> {
        if self.supports(op) {
            Ok(())
        } else {
            Err(Unsupported {
                backend: self.backend,
                operation: op,
            })
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported {
    pub backend: BackendId,
    pub operation: Operation,
}
impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} does not provide {:?} in this build",
            self.backend, self.operation
        )
    }
}
impl std::error::Error for Unsupported {}
pub const fn capabilities() -> Capabilities {
    Capabilities {
        backend: BackendId::OpenCascade,
        available: cfg!(feature = "occt"),
    }
}

/// Owned shape handle. New backends must implement dispatch here, rather than leaking
/// their concrete geometry objects into problem definitions or the UI.
#[cfg(feature = "occt")]
pub struct Brep {
    inner: crate::occt::Brep,
}
#[cfg(feature = "occt")]
impl Brep {
    fn wrap(inner: crate::occt::Brep) -> Self {
        Self { inner }
    }
    pub fn backend(&self) -> BackendId {
        BackendId::OpenCascade
    }
    pub fn extrude(profile: &Profile, z0: f64, depth: f64) -> Result<Self, String> {
        crate::occt::Brep::extrude(profile, z0, depth).map(Self::wrap)
    }
    pub fn box_from_corners(lo: V3, hi: V3) -> Self {
        Self::wrap(crate::occt::Brep::box_from_corners(lo, hi))
    }
    pub fn cylinder(base: V3, radius: f64, axis: V3, height: f64) -> Self {
        Self::wrap(crate::occt::Brep::cylinder(base, radius, axis, height))
    }
    pub fn boolean(&self, op: Bool, other: &Self) -> Result<Self, String> {
        self.inner.boolean(op, &other.inner).map(Self::wrap)
    }
    pub fn fillet_all(&self, radius: f64) -> Result<Self, String> {
        self.inner.fillet_all(radius).map(Self::wrap)
    }
    pub fn face_count(&self) -> usize {
        self.inner.face_count()
    }
    pub fn faces(&self) -> Vec<FaceInfo> {
        self.inner.faces()
    }
    pub fn bounds(&self) -> (V3, V3) {
        self.inner.bounds()
    }
    pub fn from_step_text(text: &str) -> Result<Self, String> {
        crate::occt::Brep::from_step_text(text).map(Self::wrap)
    }
    pub fn to_step_text(&self) -> Result<String, String> {
        self.inner.to_step_text()
    }
    pub fn read_step(path: &std::path::Path) -> Result<Self, String> {
        crate::occt::Brep::read_step(path).map(Self::wrap)
    }
    pub fn write_step(&self, path: &std::path::Path) -> Result<(), String> {
        self.inner.write_step(path)
    }
    pub fn tessellate(&self, deflection: f64) -> Result<SurfaceMesh, String> {
        self.inner.tessellate(deflection)
    }
    pub fn solid(&self, deflection: f64) -> Result<SurfaceSolid, String> {
        self.inner.solid(deflection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn availability_is_not_full_cad_parity() {
        let c = capabilities();
        assert_eq!(c.supports(Operation::Step), cfg!(feature = "occt"));
        for op in [
            Operation::Iges,
            Operation::Healing,
            Operation::PersistentTopology,
        ] {
            assert_eq!(c.require(op).unwrap_err().operation, op);
        }
    }
    #[cfg(feature = "occt")]
    #[test]
    fn adapter_preserves_surface_tags_and_step_roundtrip() {
        let a = Brep::box_from_corners([0.; 3], [1., 2., 3.]);
        let b = crate::occt::Brep::box_from_corners([0.; 3], [1., 2., 3.]);
        assert_eq!(a.faces(), b.faces());
        assert_eq!(a.tessellate(0.01).unwrap(), b.tessellate(0.01).unwrap());
        let back = Brep::from_step_text(&a.to_step_text().unwrap()).unwrap();
        assert_eq!(back.face_count(), 6);
        assert!((back.solid(0.01).unwrap().volume() - 6.).abs() < 1e-12);
        assert_eq!(a.backend(), BackendId::OpenCascade);
    }
}
