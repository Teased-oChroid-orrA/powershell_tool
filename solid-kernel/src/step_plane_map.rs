//! Interval source-plane parameter mapping with explicit body length units.
//! Only actual STEP PLANE definitions, never an affine guess for spline UVs.
use crate::step_frame::{direction, dot, unit, V};
use crate::{
    analytic_step::{call, vector3},
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

#[derive(Debug)]
pub struct PlaneLineLocus {
    pub face: EntityReference,
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub wrapper: EntityReference,
    pub declared_master: crate::step_pcurve::CurveMaster,
    pub master_reference: EntityReference,
    /// Source-identity comparison only; not complete master validation.
    pub checked_pcurve_is_master: bool,
    pub source_to_3d_error_mm: [f64; 2],
    pub source_to_pcurve_error_mm: [f64; 2],
    pub curve_to_pcurve_error_mm: [f64; 2],
    /// Geometric direction agreement only; not parameter speed/offset agreement.
    pub natural_direction_dot: [f64; 2],
}
fn world_point(data: &Inventory, id: u64, scale: f64) -> Result<V, Error> {
    let e = &data.entities()[&id];
    let p = match call(e, "CARTESIAN_POINT")? {
        [Value::String(_), v] => vector3(e, v)?,
        _ => return Err(fail(e, "invalid line/source vertex point")),
    };
    Ok(p.map(|x| Interval::point(x) * Interval::point(scale)))
}
fn distance_upper(a: V, b: V, e: &Entity) -> Result<f64, Error> {
    let d: V = std::array::from_fn(|i| a[i] - b[i]);
    let r = (d[0].square() + d[1].square() + d[2].square()).sqrt_nonnegative();
    if !r.finite() {
        return Err(fail(e, "line agreement distance not bounded"));
    }
    Ok(r.hi)
}
fn projection(p: V, origin: V, direction: V) -> V {
    let delta = std::array::from_fn(|i| p[i] - origin[i]);
    let t = dot(delta, direction);
    std::array::from_fn(|i| origin[i] + direction[i] * t)
}
impl PlaneParameterMap {
    fn associated_pcurve<'a>(
        &self,
        context: &'a BodyContext,
        data: &Inventory,
        edge: EntityReference,
        pcurve: EntityReference,
    ) -> Result<
        (
            &'a crate::step_topology::RawEdge,
            crate::step_pcurve::PcurveDeclaration,
            crate::step_pcurve::SurfaceCurveDeclaration,
        ),
        Error,
    > {
        let face = data.resolve_reference(self.face)?;
        data.resolve_reference(context.length_context())?;
        let entity = data.resolve_reference(edge)?;
        let f = context
            .body()
            .faces
            .get(&self.face.entity)
            .ok_or_else(|| fail(face, "map face not in this source body"))?;
        if !f
            .bounds
            .iter()
            .flat_map(|b| &b.edges)
            .any(|u| u.edge == edge.entity)
        {
            return Err(fail(entity, "edge is not a bound of the mapped face"));
        }
        let topology = context
            .body()
            .edges
            .get(&edge.entity)
            .ok_or_else(|| fail(entity, "source is not a body edge"))?;
        let declaration =
            crate::step_pcurve::read_pcurve(data, pcurve, crate::nurbs::Limits::default())?;
        if declaration.basis_surface != self.surface {
            return Err(fail(
                entity,
                "p-curve basis surface does not match the mapped face",
            ));
        }
        let mut current = topology.curve;
        let mut found = None;
        let mut seen = std::collections::BTreeSet::new();
        while seen.len() < 32 && seen.insert(current) {
            let record = &data.entities()[&current];
            if !record
                .components
                .iter()
                .any(|c| matches!(c.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE"))
            {
                break;
            }
            let wrapper = crate::step_pcurve::read_surface_curve(
                data,
                data.entity_reference(current).unwrap(),
            )?;
            current = wrapper.geometry_3d.entity;
            if wrapper.pcurves.contains(&pcurve) {
                found = Some(wrapper);
                break;
            }
        }
        let wrapper = found.ok_or_else(|| {
            fail(
                entity,
                "p-curve is not associated with this source edge wrapper",
            )
        })?;
        Ok((topology, declaration, wrapper))
    }
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
        let (topology, declaration, wrapper) =
            self.associated_pcurve(context, data, edge, pcurve)?;
        let crate::step_pcurve::ParametricCurve::Line { origin: uv, .. } = declaration.geometry
        else {
            return Err(fail(entity, "only plane LINE p-curve loci are certified"));
        };
        let geometry = crate::analytic_step::curve_geometry(data, topology.curve)?;
        let (origin, vector) = match call(geometry, "LINE")? {
            [Value::String(_), Value::Ref(p), Value::Ref(v)] => (*p, &data.entities()[v]),
            _ => return Err(fail(geometry, "only 3D LINE loci are certified")),
        };
        let direction = match call(vector, "VECTOR")? {
            [Value::String(_), Value::Ref(d), m]
                if crate::analytic_step::scalar(vector, m)? > 0. =>
            {
                direction(data, &Value::Ref(*d), [0.; 3], vector)?
            }
            _ => return Err(fail(vector, "invalid 3D line vector")),
        };
        let origin = world_point(data, origin, self.scale_mm)?;
        let pc_origin = self.point_bounds(uv)?;
        let pc_origin: V =
            std::array::from_fn(|i| Interval::new(pc_origin.min[i], pc_origin.max[i]));
        // Use original direction ratios for the proof, not the rounded display
        // normalization retained in the parameter declaration.
        let pc = &data.entities()[&declaration.geometry_source.entity];
        let vector = match call(pc, "LINE")? {
            [Value::String(_), Value::Ref(_), Value::Ref(v)] => &data.entities()[v],
            _ => return Err(fail(pc, "invalid parameter LINE")),
        };
        let d = match call(vector, "VECTOR")? {
            [Value::String(_), Value::Ref(d), _] => &data.entities()[d],
            _ => return Err(fail(vector, "invalid parameter vector")),
        };
        let raw = match call(d, "DIRECTION")? {
            [Value::String(_), Value::List(v)] if v.len() == 2 => [
                crate::analytic_step::scalar(d, &v[0])?,
                crate::analytic_step::scalar(d, &v[1])?,
            ],
            _ => return Err(fail(d, "invalid parameter direction")),
        };
        let raw = unit(
            [
                Interval::point(raw[0]),
                Interval::point(raw[1]),
                Interval::point(0.),
            ],
            d,
        )?;
        let pc_direction = unit(
            std::array::from_fn(|i| self.x[i] * raw[0] + self.y[i] * raw[1]),
            d,
        )?;
        let mut source_to_3d = [0.; 2];
        let mut source_to_pc = [0.; 2];
        let mut curve_to_pc = [0.; 2];
        let mut projected = [[Interval::point(0.); 3]; 2];
        for (i, id) in [topology.start, topology.end].into_iter().enumerate() {
            let vertex = &context.body().vertices[&id];
            let point = world_point(data, vertex.point, self.scale_mm)?;
            let q = projection(point, origin, direction);
            let r = projection(point, pc_origin, pc_direction);
            let s = projection(q, pc_origin, pc_direction);
            source_to_3d[i] = distance_upper(point, q, entity)?;
            source_to_pc[i] = distance_upper(point, r, entity)?;
            curve_to_pc[i] = distance_upper(q, s, entity)?;
            projected[i] = q;
            if [source_to_3d[i], source_to_pc[i], curve_to_pc[i]]
                .iter()
                .any(|x| *x > tolerance_mm)
            {
                return Err(fail(
                    entity,
                    "finite 3D line/p-curve/source vertex locus agreement not certified",
                ));
            }
        }
        let span: V = std::array::from_fn(|i| projected[1][i] - projected[0][i]);
        let sense = dot(span, direction);
        if (topology.same_sense && sense.lo <= 0.) || (!topology.same_sense && sense.hi >= 0.) {
            return Err(fail(entity, "source line extent/sense not certified"));
        }
        let orientation = dot(direction, pc_direction);
        if !orientation.finite() {
            return Err(fail(entity, "line natural orientation not bounded"));
        }
        let master = match wrapper.declared_master {
            crate::step_pcurve::CurveMaster::Curve3d => wrapper.geometry_3d,
            crate::step_pcurve::CurveMaster::Pcurve1 => wrapper.pcurves[0],
            crate::step_pcurve::CurveMaster::Pcurve2 => wrapper.pcurves[1],
        };
        Ok(PlaneLineLocus {
            face: self.face,
            edge,
            pcurve,
            wrapper: wrapper.source,
            declared_master: wrapper.declared_master,
            master_reference: master,
            checked_pcurve_is_master: master == pcurve,
            source_to_3d_error_mm: source_to_3d,
            source_to_pcurve_error_mm: source_to_pc,
            curve_to_pcurve_error_mm: curve_to_pc,
            natural_direction_dot: [orientation.lo, orientation.hi],
        })
    }
}

/// Unsigned whole-circle carrier comparison. It does not select an arc,
/// establish edge sense, or certify complete master/trim/body agreement.
#[derive(Debug)]
pub struct PlaneCircleCarrier {
    pub face: EntityReference,
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub wrapper: EntityReference,
    pub declared_master: crate::step_pcurve::CurveMaster,
    pub master_reference: EntityReference,
    pub centre_error_mm: f64,
    pub radius_error_mm: f64,
    pub normal_error: f64,
    pub hausdorff_upper_mm: f64,
    pub natural_normal_dot: [f64; 2],
}
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
        use crate::step_frame::{cross, SourceFrame};
        let entity = data.resolve_reference(edge)?;
        if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
            return Err(fail(
                entity,
                "circle carrier tolerance must be finite and positive",
            ));
        }
        let (topology, declaration, wrapper) =
            self.associated_pcurve(context, data, edge, pcurve)?;
        let crate::step_pcurve::ParametricCurve::Circle { origin, radius, .. } =
            declaration.geometry
        else {
            return Err(fail(
                entity,
                "requires a parameter CIRCLE on the source plane",
            ));
        };
        let geometry = crate::analytic_step::curve_geometry(data, topology.curve)?;
        let (placement, source_radius) = match call(geometry, "CIRCLE")? {
            [Value::String(_), Value::Ref(frame), r] => (
                &data.entities()[frame],
                crate::analytic_step::scalar(geometry, r)?,
            ),
            _ => return Err(fail(geometry, "requires a source 3D CIRCLE carrier")),
        };
        let source_radius = Interval::point(source_radius) * Interval::point(self.scale_mm);
        let radius = Interval::point(radius) * Interval::point(self.scale_mm);
        if !source_radius.finite()
            || source_radius.lo <= 0.
            || source_radius.hi > 1e30
            || !radius.finite()
            || radius.lo <= 0.
            || radius.hi > 1e30
        {
            return Err(fail(geometry, "circle radii must be positively bounded"));
        }
        let frame = SourceFrame::read(data, placement, self.scale_mm)?;
        let bounds = self.point_bounds(origin)?;
        let mapped: V = std::array::from_fn(|i| Interval::new(bounds.min[i], bounds.max[i]));
        let centre_error = distance_upper(frame.origin, mapped, entity)?;
        let plane_normal = cross(self.x, self.y);
        let natural = dot(frame.z, plane_normal);
        // Carrier orientation is unsigned; choose a normal sign only when its
        // alignment is certified. Arc direction and parameter offset stay open.
        let aligned = if natural.lo > 0. {
            plane_normal
        } else if natural.hi < 0. {
            plane_normal.map(|x| Interval::point(0.) - x)
        } else {
            return Err(fail(
                entity,
                "circle carrier normal alignment not certified",
            ));
        };
        let normal_error = distance_upper(frame.z, aligned, entity)?;
        let difference = source_radius - radius;
        let radius_error = difference.lo.abs().max(difference.hi.abs());
        let largest = source_radius.hi.max(radius.hi);
        // For aligned unit normals, the minimal plane rotation has operator
        // norm ||n-m|| = 2 sin(theta/2). Pairing equally rotated points bounds
        // both directed distances by centre + radius + Rmax*normal error.
        let total = Interval::point(centre_error)
            + Interval::point(radius_error)
            + Interval::point(largest) * Interval::point(normal_error);
        if !total.finite() || total.hi > tolerance_mm {
            return Err(fail(entity, "whole-circle carrier agreement not certified"));
        }
        let master_reference = match wrapper.declared_master {
            crate::step_pcurve::CurveMaster::Curve3d => wrapper.geometry_3d,
            crate::step_pcurve::CurveMaster::Pcurve1 => wrapper.pcurves[0],
            crate::step_pcurve::CurveMaster::Pcurve2 => wrapper.pcurves[1],
        };
        Ok(PlaneCircleCarrier {
            face: self.face,
            edge,
            pcurve,
            wrapper: wrapper.source,
            declared_master: wrapper.declared_master,
            master_reference,
            centre_error_mm: centre_error,
            radius_error_mm: radius_error,
            normal_error,
            hausdorff_upper_mm: total.hi,
            natural_normal_dot: [natural.lo, natural.hi],
        })
    }
}
