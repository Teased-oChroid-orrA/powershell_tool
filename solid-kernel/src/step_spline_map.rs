//! Source-bound rational surface parameter enclosures. Raw UV/knots remain
//! unscaled; only Cartesian surface coordinates convert to physical millimetres.
//! No projection uniqueness, regularity, trim, master or body validity claim.
use crate::{
    geometry_bounds::Bounds3,
    native_interval::Interval as I,
    nurbs,
    nurbs_surface::{Limits, NurbsSurface},
    step::{EntityReference, Error, ErrorKind, Inventory},
    step_body::BodyContext,
    step_pcurve,
};
#[derive(Debug)]
pub struct SplineParameterMap {
    face: EntityReference,
    source: EntityReference,
    context: EntityReference,
    geometry_source_units: NurbsSurface,
    scale_mm: f64,
    offset: usize,
}
impl SplineParameterMap {
    pub fn read(
        context: &BodyContext,
        data: &Inventory,
        face: EntityReference,
        limits: Limits,
    ) -> Result<Self, Error> {
        let source = context.spline_surface_source(data, face)?;
        // Decode with an identity Cartesian factor. Keep source binary64
        // controls in the evaluator; actual body scaling occurs outward after
        // enclosure, preserving source-to-world conversion rounding as well.
        // Raw control arithmetic must satisfy the evaluator's explicit domain.
        let declaration = crate::step_nurbs_surface::read_surface(data, source.entity, 1., limits)?;
        Ok(Self {
            face,
            source,
            context: context.length_context(),
            geometry_source_units: declaration.surface,
            scale_mm: context.millimetres_per_source_unit(),
            offset: data.resolve_reference(source)?.offset,
        })
    }
    pub fn face_reference(&self) -> EntityReference {
        self.face
    }
    pub fn surface_reference(&self) -> EntityReference {
        self.source
    }
    pub fn length_context(&self) -> EntityReference {
        self.context
    }
    pub fn millimetres_per_source_unit(&self) -> f64 {
        self.scale_mm
    }
    /// Cartesian control coordinates are in source length units, not mm. UV
    /// domains and knots are abstract raw parameters, never body length values.
    pub fn geometry_source_units(&self) -> &NurbsSurface {
        &self.geometry_source_units
    }
    pub fn bounds(&self, uv: [[f64; 2]; 2], max_patches: usize) -> Result<Bounds3, Error> {
        let b = self
            .geometry_source_units
            .enclose(uv, max_patches)
            .map_err(|mut e| {
                e.offset = self.offset;
                e
            })?;
        let world: [I; 3] =
            std::array::from_fn(|i| I::new(b.min[i], b.max[i]) * I::point(self.scale_mm));
        if world
            .iter()
            .any(|x| !x.finite() || x.lo < -1e30 || x.hi > 1e30)
        {
            return Err(Error {
                offset: self.offset,
                kind: ErrorKind::Geometry,
                detail: "spline mapping exceeds bounded world geometry".into(),
            });
        }
        Ok(Bounds3 {
            min: world.map(|x| x.lo),
            max: world.map(|x| x.hi),
        })
    }
    pub fn point_bounds(&self, uv: [f64; 2], max_patches: usize) -> Result<Bounds3, Error> {
        self.bounds(uv.map(|x| [x, x]), max_patches)
    }
    /// Bound a whole LINE/CIRCLE/spline p-curve interval on this source surface.
    /// The UV rectangle must lie in its domain; do not clamp/wrap/renormalize.
    /// CIRCLE parameters use an explicitly bounded [-8,8] radian interval.
    pub fn pcurve_bounds(
        &self,
        data: &Inventory,
        pcurve: EntityReference,
        parameters: [f64; 2],
        curve_limits: nurbs::Limits,
        max_curve_spans: usize,
        max_surface_patches: usize,
    ) -> Result<Bounds3, Error> {
        data.resolve_reference(self.face)?;
        data.resolve_reference(self.context)?;
        let declaration = step_pcurve::read_pcurve(data, pcurve, curve_limits)?;
        self.bounds_for_declaration(
            data,
            &declaration,
            parameters,
            max_curve_spans,
            max_surface_patches,
        )
    }
    pub(crate) fn bounds_for_declaration(
        &self,
        data: &Inventory,
        declaration: &step_pcurve::PcurveDeclaration,
        parameters: [f64; 2],
        max_curve_spans: usize,
        max_surface_patches: usize,
    ) -> Result<Bounds3, Error> {
        crate::step_parameter_map::ParameterMapRef::Spline(self).bounds_for_declaration(
            data,
            declaration,
            parameters,
            max_curve_spans,
            max_surface_patches,
        )
    }
}
