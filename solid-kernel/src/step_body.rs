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
