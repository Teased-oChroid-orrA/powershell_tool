//! Strict source-local body/unit binding; not a geometry or solid certificate.
use crate::step::{EntityReference, Error, ErrorKind, Inventory, RawBody, Value};

/// One directly represented, unplaced body and its declared length context.
/// Assembly mappings and ambiguous representations refuse. Coordinates in
/// `body()` remain source units; the factor converts lengths to millimetres.
#[derive(Debug)]
pub struct BodyContext {
    body: RawBody,
    context: EntityReference,
    scale_mm: f64,
}
impl BodyContext {
    pub fn read(data: &Inventory) -> Result<Self, Error> {
        let fail = |s: &str| Error {
            offset: 0,
            kind: ErrorKind::Geometry,
            detail: s.into(),
        };
        for e in data.entities().values() {
            if e.components.iter().any(|c| {
                matches!(
                    c.name.as_str(),
                    "MAPPED_ITEM"
                        | "NEXT_ASSEMBLY_USAGE_OCCURRENCE"
                        | "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION"
                        | "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION"
                )
            }) {
                return Err(error(e, "assembly/placement mapping is not implemented"));
            }
        }
        if data.counts().get("MANIFOLD_SOLID_BREP") != Some(&1) {
            return Err(fail(
                "native context scope requires exactly one supported body",
            ));
        }
        let body = data.raw_bodies()?.remove(0);
        let mut context = None;
        for e in data.entities().values() {
            for c in &e.components {
                if matches!(
                    c.name.as_str(),
                    "ADVANCED_BREP_SHAPE_REPRESENTATION" | "SHAPE_REPRESENTATION"
                ) {
                    if let [Value::String(_), Value::List(items), Value::Ref(ctx)] =
                        c.args.as_slice()
                    {
                        if items.iter().any(|v| *v == Value::Ref(body.id)) {
                            if context.replace(*ctx).is_some() {
                                return Err(error(
                                    e,
                                    "multiple body representations require explicit mapping",
                                ));
                            }
                            for item in items {
                                if let Value::Ref(id) = item {
                                    let ie = &data.entities()[id];
                                    if *id != body.id
                                        && !ie
                                            .components
                                            .iter()
                                            .all(|c| c.name == "AXIS2_PLACEMENT_3D")
                                    {
                                        return Err(error(ie, "unsupported representation item"));
                                    }
                                } else {
                                    return Err(error(
                                        e,
                                        "unsupported representation item encoding",
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        let context = context.ok_or_else(|| fail("no direct body/representation association"))?;
        let contexts = data.length_contexts()?;
        let scale = contexts
            .iter()
            .find(|c| c.context == context)
            .ok_or_else(|| fail("body representation has no assigned length context"))?
            .metres
            * 1000.;
        if !scale.is_finite() || scale <= 0. {
            return Err(fail("source length normalization overflows"));
        }
        Ok(Self {
            body,
            context: data
                .entity_reference(context)
                .ok_or_else(|| fail("missing body length context entity"))?,
            scale_mm: scale,
        })
    }
    pub fn body(&self) -> &RawBody {
        &self.body
    }
    pub fn length_context(&self) -> EntityReference {
        self.context
    }
    pub fn millimetres_per_source_unit(&self) -> f64 {
        self.scale_mm
    }
    /// Extract the spline declaration of a face in this exact source body,
    /// using its assigned length context. This does not validate trims, geometry
    /// regularity or the full B-Rep. Analytic surfaces refuse this spline-only API.
    pub fn read_spline_surface(
        &self,
        data: &Inventory,
        face: EntityReference,
        limits: crate::nurbs_surface::Limits,
    ) -> Result<crate::step_nurbs_surface::StepSurfaceDeclaration, Error> {
        let source = self.spline_surface_source(data, face)?;
        crate::step_nurbs_surface::read_surface(data, source.entity, self.scale_mm, limits)
    }
    pub(crate) fn spline_surface_source(
        &self,
        data: &Inventory,
        face: EntityReference,
    ) -> Result<EntityReference, Error> {
        // Resolve both references before looking up numeric topology identifiers.
        data.resolve_reference(self.context)?;
        let entity = data.resolve_reference(face)?;
        if face.document != self.context.document {
            return Err(error(
                entity,
                "face belongs to a different source body document",
            ));
        }
        let face = self
            .body
            .faces
            .get(&face.entity)
            .ok_or_else(|| error(entity, "source reference is not a face of this body"))?;
        Ok(data.entity_reference(face.surface).unwrap())
    }
    /// Read an edge's terminal spline declaration using this body's source
    /// units. Wrapper traversal is bounded; p-curves, trims and edge agreement
    /// are not certified by declaration extraction.
    pub fn read_spline_curve(
        &self,
        data: &Inventory,
        edge: EntityReference,
        limits: crate::nurbs::Limits,
    ) -> Result<crate::step_nurbs::StepSplineDeclaration, Error> {
        data.resolve_reference(self.context)?;
        let entity = data.resolve_reference(edge)?;
        let edge = self
            .body
            .edges
            .get(&edge.entity)
            .ok_or_else(|| error(entity, "source reference is not an edge of this body"))?;
        let geometry = crate::analytic_step::curve_geometry(data, edge.curve)?;
        crate::step_nurbs::read_curve(data, geometry.id, self.scale_mm, limits)
    }
    /// [`Self::read_spline_curve`] for an edge whose curve may be unclamped (a periodic spline): evaluated on its active
    /// domain, see `step_nurbs::read_curve_active_domain`.
    pub fn read_spline_curve_active_domain(
        &self,
        data: &Inventory,
        edge: EntityReference,
        limits: crate::nurbs::Limits,
    ) -> Result<crate::step_nurbs::StepSplineDeclaration, Error> {
        data.resolve_reference(self.context)?;
        let entity = data.resolve_reference(edge)?;
        let edge = self
            .body
            .edges
            .get(&edge.entity)
            .ok_or_else(|| error(entity, "source reference is not an edge of this body"))?;
        let geometry = crate::analytic_step::curve_geometry(data, edge.curve)?;
        crate::step_nurbs::read_curve_active_domain(data, geometry.id, self.scale_mm, limits)
    }
    pub(crate) fn into_parts(self) -> (RawBody, f64) {
        (self.body, self.scale_mm)
    }
}
fn error(e: &crate::step::Entity, detail: &str) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: detail.into(),
    }
}
