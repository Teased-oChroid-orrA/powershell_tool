//! Borrowed source parameter-map dispatch for shared bounded world queries.
//! Maps retain their own unit/domain rules; no surface is approximated by another.
use crate::{
    geometry_bounds::Bounds3,
    step::{EntityReference, Error, ErrorKind, Inventory},
    step_cylinder_map::CylinderParameterMap,
    step_pcurve::{self, PcurveDeclaration},
    step_plane_map::PlaneParameterMap,
    step_spline_map::SplineParameterMap,
};
#[derive(Debug, Clone, Copy)]
pub enum ParameterMapRef<'a> {
    Plane(&'a PlaneParameterMap),
    Cylinder(&'a CylinderParameterMap),
    Spline(&'a SplineParameterMap),
}
impl<'a> From<&'a PlaneParameterMap> for ParameterMapRef<'a> {
    fn from(map: &'a PlaneParameterMap) -> Self {
        Self::Plane(map)
    }
}
impl<'a> From<&'a CylinderParameterMap> for ParameterMapRef<'a> {
    fn from(map: &'a CylinderParameterMap) -> Self {
        Self::Cylinder(map)
    }
}
impl<'a> From<&'a SplineParameterMap> for ParameterMapRef<'a> {
    fn from(map: &'a SplineParameterMap) -> Self {
        Self::Spline(map)
    }
}
impl ParameterMapRef<'_> {
    pub fn face_reference(self) -> EntityReference {
        match self {
            Self::Plane(m) => m.face_reference(),
            Self::Cylinder(m) => m.face_reference(),
            Self::Spline(m) => m.face_reference(),
        }
    }
    pub fn surface_reference(self) -> EntityReference {
        match self {
            Self::Plane(m) => m.surface_reference(),
            Self::Cylinder(m) => m.surface_reference(),
            Self::Spline(m) => m.surface_reference(),
        }
    }
    pub fn length_context(self) -> EntityReference {
        match self {
            Self::Plane(m) => m.length_context(),
            Self::Cylinder(m) => m.length_context(),
            Self::Spline(m) => m.length_context(),
        }
    }
    /// World millimetres with the source map's original raw parameter semantics.
    /// Patch budgets apply to spline surfaces; analytic maps use no spline patches.
    pub fn bounds(self, uv: [[f64; 2]; 2], max_surface_patches: usize) -> Result<Bounds3, Error> {
        match self {
            Self::Plane(m) => m.bounds(uv),
            Self::Cylinder(m) => m.bounds(uv),
            Self::Spline(m) => m.bounds(uv, max_surface_patches),
        }
    }
    pub(crate) fn bounds_for_declaration(
        self,
        data: &Inventory,
        declaration: &PcurveDeclaration,
        parameters: [f64; 2],
        max_curve_spans: usize,
        max_surface_patches: usize,
    ) -> Result<Bounds3, Error> {
        let entity = data.resolve_reference(declaration.source)?;
        if declaration.basis_surface != self.surface_reference() {
            return Err(Error {
                offset: entity.offset,
                kind: ErrorKind::Geometry,
                detail: "p-curve basis does not match source parameter map".into(),
            });
        }
        let uv = step_pcurve::parameter_bounds(data, declaration, parameters, max_curve_spans)?;
        self.bounds(uv, max_surface_patches).map_err(|mut e| {
            e.offset = entity.offset;
            e
        })
    }
}
