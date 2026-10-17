//! Interval source-plane parameter mapping with explicit body length units.
//! Only actual STEP PLANE definitions, never an affine guess for spline UVs.
use crate::step_frame::{unit, V};
use crate::{
    analytic_step::call,
    geometry_bounds::Bounds3,
    native_interval::Interval,
    step::{Entity, EntityReference, Error, ErrorKind, Inventory, Value},
    step_body::BodyContext,
};
fn fail(e: &Entity, s: &str) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: s.into(),
    }
}
#[derive(Debug)]
pub struct PlaneParameterMap {
    face: EntityReference,
    surface: EntityReference,
    context: EntityReference,
    origin: V,
    x: V,
    y: V,
    scale_mm: f64,
    offset: usize,
}
impl PlaneParameterMap {
    pub fn read(
        context: &BodyContext,
        data: &Inventory,
        face: EntityReference,
    ) -> Result<Self, Error> {
        data.resolve_reference(context.length_context())?;
        let e = data.resolve_reference(face)?;
        let topology = context
            .body()
            .faces
            .get(&face.entity)
            .ok_or_else(|| fail(e, "source reference is not a face of this body"))?;
        let s = &data.entities()[&topology.surface];
        let frame = match call(s, "PLANE")? {
            [Value::String(_), Value::Ref(frame)] => &data.entities()[frame],
            _ => {
                return Err(fail(
                    s,
                    "source parameter map requires an actual STEP PLANE",
                ))
            }
        };
        let scale_mm = context.millimetres_per_source_unit();
        let placement = crate::step_frame::SourceFrame::read(data, frame, scale_mm)?;
        let crate::step_frame::SourceFrame { origin, x, y, .. } = placement;
        Ok(Self {
            face,
            surface: data.entity_reference(topology.surface).unwrap(),
            context: context.length_context(),
            origin,
            x,
            y,
            scale_mm,
            offset: s.offset,
        })
    }
    pub fn face_reference(&self) -> EntityReference {
        self.face
    }
    pub fn surface_reference(&self) -> EntityReference {
        self.surface
    }
    pub fn length_context(&self) -> EntityReference {
        self.context
    }
    pub fn millimetres_per_source_unit(&self) -> f64 {
        self.scale_mm
    }
    /// Raw plane U/V are source length parameters. Both scale by the bound body
    /// length factor; no angle/spline-knot parameter inference occurs here.
    pub fn bounds(&self, parameters: [[f64; 2]; 2]) -> Result<Bounds3, Error> {
        let error = |s: &str| Error {
            offset: self.offset,
            kind: ErrorKind::Geometry,
            detail: s.into(),
        };
        if parameters
            .iter()
            .any(|p| p.iter().any(|x| !x.is_finite()) || p[0] > p[1])
        {
            return Err(error("invalid source-plane parameter rectangle"));
        }
        let uv = parameters.map(|p| Interval::new(p[0], p[1]) * Interval::point(self.scale_mm));
        let result: V =
            std::array::from_fn(|i| self.origin[i] + self.x[i] * uv[0] + self.y[i] * uv[1]);
        if result
            .iter()
            .any(|x| !x.finite() || x.lo < -1e30 || x.hi > 1e30)
        {
            return Err(error(
                "source-plane mapping exceeds bounded geometry domain",
            ));
        }
        Ok(Bounds3 {
            min: result.map(|x| x.lo),
            max: result.map(|x| x.hi),
        })
    }
    /// Enclose the whole supported spline p-curve interval in world millimetres.
    /// This checks exact-document identity and the basis surface, but does not
    /// certify edge association, master agreement, trim validity or injectivity.
    /// `max_spans` is a hard enclosure budget; no partial result is returned.
    pub fn spline_pcurve_bounds(
        &self,
        data: &Inventory,
        pcurve: EntityReference,
        parameters: [f64; 2],
        limits: crate::nurbs::Limits,
        max_spans: usize,
    ) -> Result<Bounds3, Error> {
        data.resolve_reference(self.face)?;
        let entity = data.resolve_reference(pcurve)?;
        let declaration = crate::step_pcurve::read_pcurve(data, pcurve, limits)?;
        if declaration.basis_surface != self.surface {
            return Err(fail(
                entity,
                "spline p-curve basis does not match source plane",
            ));
        }
        let crate::step_pcurve::ParametricCurve::Spline(spline) = declaration.geometry else {
            return Err(fail(
                entity,
                "source-plane spline enclosure requires a spline p-curve",
            ));
        };
        let bounds = spline
            .curve()
            .enclose(parameters, max_spans)
            .map_err(|mut error| {
                error.offset = entity.offset;
                error
            })?;
        // UV is raw source length; only the affine plane map applies the bound
        // body unit conversion. Spline parameters/knots are never rescaled.
        self.bounds([
            [bounds.min[0], bounds.max[0]],
            [bounds.min[1], bounds.max[1]],
        ])
    }
    pub fn point_bounds(&self, parameters: [f64; 2]) -> Result<Bounds3, Error> {
        self.bounds(parameters.map(|x| [x, x]))
    }
}

/// Compatibility name for the shared finite source-line report.
pub type PlaneLineLocus = crate::step_curve_agreement::LineLocus;
impl PlaneParameterMap {
    /// Certify unsigned finite line-locus agreement at the actual source edge.
    /// The target must be an associated p-curve on this actual PLANE. Other
    /// p-curves/master representations, rates/offsets and general trims remain
    /// unverified; the report explicitly identifies which master was declared.
    pub fn verify_line_locus(
        &self,
        context: &BodyContext,
        data: &Inventory,
        edge: EntityReference,
        pcurve: EntityReference,
        tolerance_mm: f64,
    ) -> Result<PlaneLineLocus, Error> {
        let entity = data.resolve_reference(edge)?;
        if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
            return Err(fail(
                entity,
                "line agreement tolerance must be finite and positive",
            ));
        }
        let (_topology, declaration, wrapper) = crate::step_pcurve::associated_edge_pcurve(
            context,
            data,
            self.face,
            self.surface,
            edge,
            pcurve,
        )?;
        let crate::step_pcurve::ParametricCurve::Line { origin: uv, .. } = declaration.geometry
        else {
            return Err(fail(entity, "only plane LINE p-curve loci are certified"));
        };
        let pc_origin = self.point_bounds(uv)?;
        let pc_origin: V =
            std::array::from_fn(|i| Interval::new(pc_origin.min[i], pc_origin.max[i]));
        let raw = crate::step_pcurve::line_direction_ratios(data, declaration.geometry_source)?;
        let raw = unit(
            [
                Interval::point(raw[0]),
                Interval::point(raw[1]),
                Interval::point(0.),
            ],
            entity,
        )?;
        let pc_direction = unit(
            std::array::from_fn(|i| self.x[i] * raw[0] + self.y[i] * raw[1]),
            entity,
        )?;
        crate::step_curve_agreement::verify_line(
            context,
            data,
            self.face,
            edge,
            pcurve,
            &wrapper,
            pc_origin,
            pc_direction,
            tolerance_mm,
        )
    }
}

/// Compatibility name for the shared unsigned circle-carrier report.
pub type PlaneCircleCarrier = crate::step_curve_agreement::CircleCarrier;
impl PlaneParameterMap {
    /// Bound Hausdorff distance between the entire source 3D CIRCLE and the
    /// entire plane-mapped parameter CIRCLE, retaining declared master identity.
    pub fn verify_circle_carrier(
        &self,
        context: &BodyContext,
        data: &Inventory,
        edge: EntityReference,
        pcurve: EntityReference,
        tolerance_mm: f64,
    ) -> Result<PlaneCircleCarrier, Error> {
        use crate::step_frame::cross;
        let entity = data.resolve_reference(edge)?;
        if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
            return Err(fail(
                entity,
                "circle carrier tolerance must be finite and positive",
            ));
        }
        let (_topology, declaration, wrapper) = crate::step_pcurve::associated_edge_pcurve(
            context,
            data,
            self.face,
            self.surface,
            edge,
            pcurve,
        )?;
        let crate::step_pcurve::ParametricCurve::Circle { origin, radius, .. } =
            declaration.geometry
        else {
            return Err(fail(
                entity,
                "requires a parameter CIRCLE on the source plane",
            ));
        };
        let radius = Interval::point(radius) * Interval::point(self.scale_mm);
        let bounds = self.point_bounds(origin)?;
        let mapped: V = std::array::from_fn(|i| Interval::new(bounds.min[i], bounds.max[i]));
        crate::step_curve_agreement::verify_circle(
            context,
            data,
            self.face,
            edge,
            pcurve,
            &wrapper,
            mapped,
            cross(self.x, self.y),
            radius,
            tolerance_mm,
        )
    }
}
