//! Strict source-local body/unit binding; not a geometry or solid certificate.
use crate::step::{EntityReference, Error, ErrorKind, Inventory, RawBody, Value};
use crate::step_assembly::Placement;
use std::sync::Arc;

/// One body and its declared length context. A body that product structure places (assembly usages or mapped items that place
/// shape, `step_assembly`) is read from a copy of the document moved by its rigid placement (`Inventory::rigidly_moved`), so its
/// coordinates are the root's; [`Self::document`] is that copy. Ambiguous representations refuse. Coordinates in `body()` are
/// source length units; the factor converts lengths to millimetres.
#[derive(Debug)]
pub struct BodyContext {
    body: RawBody,
    context: EntityReference,
    scale_mm: f64,
    placement: Placement,
    placed: Option<Arc<Inventory>>,
}
impl BodyContext {
    pub fn read(data: &Inventory) -> Result<Self, Error> {
        let one = || Error {
            offset: 0,
            kind: ErrorKind::Geometry,
            detail: "native context scope requires exactly one supported body".into(),
        };
        if data.counts().get("MANIFOLD_SOLID_BREP") != Some(&1) {
            return Err(one());
        }
        // one solid can still be instanced several times by the product structure
        let mut all = Self::read_all(data)?;
        if all.len() != 1 {
            return Err(one());
        }
        Ok(all.remove(0))
    }
    /// Every body of the document, each bound to its own representation and length context. Without product structure that
    /// places shape this is every `MANIFOLD_SOLID_BREP`, in the order of `Inventory::raw_bodies` (a multi-body part). With it,
    /// one context per instance in the order of `step_assembly::read`, each moved by its placement into the root's coordinates;
    /// whatever that reader refuses (cycles, non-rigid frames, ambiguous transformation direction, several roots, ...) refuses
    /// here, and so does a solid that no instance places. `MAPPED_ITEM`s that only show a representation in an annotation view
    /// (`DRAUGHTING_MODEL`, PMI) place no shape and are left out. The bodies are independent: nothing here says whether they
    /// touch or overlap.
    pub fn read_all(data: &Inventory) -> Result<Vec<Self>, Error> {
        let fail = |s: String| Error {
            offset: 0,
            kind: ErrorKind::Geometry,
            detail: s,
        };
        if data.counts().get("MANIFOLD_SOLID_BREP").is_none_or(|n| *n == 0) {
            return Err(fail("native context scope requires a supported body".into()));
        }
        let bodies = data.raw_bodies()?;
        let contexts = data.length_contexts()?;
        if !crate::step_assembly::places_shape(data)? {
            return bodies
                .into_iter()
                .map(|body| Self::bind(data, body, &contexts))
                .collect();
        }
        let assembly = crate::step_assembly::read(data)?;
        if let Some(b) = bodies
            .iter()
            .find(|b| !assembly.parts.iter().any(|p| p.solid == b.id))
        {
            return Err(fail(format!(
                "solid #{} is not placed by the product structure",
                b.id
            )));
        }
        assembly
            .parts
            .iter()
            .map(|part| {
                if part.placement == Placement::IDENTITY {
                    let body = bodies.iter().find(|b| b.id == part.solid).unwrap().clone();
                    return Self::bind(data, body, &contexts);
                }
                let p = part.placement;
                let placed = Arc::new(data.rigidly_moved(p.rotation, p.translation)?);
                let body = placed
                    .raw_bodies()?
                    .into_iter()
                    .find(|b| b.id == part.solid)
                    .unwrap();
                let mut context = Self::bind(&placed, body, &contexts)?;
                context.placement = p;
                context.placed = Some(placed);
                Ok(context)
            })
            .collect()
    }
    fn bind(
        data: &Inventory,
        body: RawBody,
        contexts: &[crate::step::LengthContext],
    ) -> Result<Self, Error> {
        let fail = |s: &str| Error {
            offset: 0,
            kind: ErrorKind::Geometry,
            detail: s.into(),
        };
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
                            // Several representations may list the solid (AP203 validation properties, shape aspects, the
                            // source of a PMI view's map): in one context they share one coordinate space and unit.
                            if context.replace(*ctx).is_some_and(|c| c != *ctx) {
                                return Err(error(
                                    e,
                                    "multiple body representations in different contexts require explicit mapping",
                                ));
                            }
                            for item in items {
                                if let Value::Ref(id) = item {
                                    let ie = &data.entities()[id];
                                    if *id != body.id
                                        && !ie.components.iter().all(|c| {
                                            matches!(
                                                c.name.as_str(),
                                                // a curve set (datum targets, supplemental wireframe) has no volume
                                                "AXIS2_PLACEMENT_3D"
                                                    | "MANIFOLD_SOLID_BREP"
                                                    | "GEOMETRIC_CURVE_SET"
                                            )
                                        })
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
            placement: Placement::IDENTITY,
            placed: None,
        })
    }
    /// The document this body is read from: the moved copy for a placed instance, else `source` (the document it was read from).
    pub fn document<'a>(&'a self, source: &'a Inventory) -> &'a Inventory {
        self.placed.as_deref().unwrap_or(source)
    }
    /// The rigid placement that carried the body into the root's coordinates (identity for an unplaced body), in source units.
    pub fn placement(&self) -> Placement {
        self.placement
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
        let data = self.document(data);
        let source = self.spline_surface_source(data, face)?;
        crate::step_nurbs_surface::read_surface(data, source.entity, self.scale_mm, limits)
    }
    /// [`Self::read_spline_surface`] for a face whose surface may be unclamped (periodic): evaluated on its active domain.
    pub fn read_spline_surface_active_domain(
        &self,
        data: &Inventory,
        face: EntityReference,
        limits: crate::nurbs_surface::Limits,
    ) -> Result<crate::step_nurbs_surface::StepSurfaceDeclaration, Error> {
        let data = self.document(data);
        let source = self.spline_surface_source(data, face)?;
        crate::step_nurbs_surface::read_surface_active_domain(data, source.entity, self.scale_mm, limits)
    }
    pub(crate) fn spline_surface_source(
        &self,
        data: &Inventory,
        face: EntityReference,
    ) -> Result<EntityReference, Error> {
        let data = self.document(data);
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
        let data = self.document(data);
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
        let data = self.document(data);
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
    pub(crate) fn placed_document(&self) -> Option<Arc<Inventory>> {
        self.placed.clone()
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
