//! Interval source-plane parameter mapping with explicit body length units.
//! Only actual STEP PLANE definitions, never an affine guess for spline UVs.
use crate::{
    analytic_step::{call, vector3},
    geometry_bounds::Bounds3,
    native_interval::Interval,
    step::{Entity, EntityReference, Error, ErrorKind, Inventory, Value},
    step_body::BodyContext,
};
type V = [Interval; 3];
fn fail(e: &Entity, s: &str) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: s.into(),
    }
}
fn dot(a: V, b: V) -> Interval {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}
fn unit(v: V, e: &Entity) -> Result<V, Error> {
    let m = v
        .iter()
        .map(|x| x.lo.abs().max(x.hi.abs()))
        .fold(0., f64::max);
    if !m.is_finite() || m == 0. {
        return Err(fail(e, "zero/nonfinite source-plane direction"));
    }
    let v = v.map(|x| x / Interval::point(m));
    let n = (v[0].square() + v[1].square() + v[2].square()).sqrt_nonnegative();
    if !n.finite() || n.lo <= 0. {
        return Err(fail(e, "source-plane direction norm not certified"));
    }
    let v = v.map(|x| x / n);
    if v.iter().any(|x| !x.finite()) {
        return Err(fail(e, "unresolved source-plane normalization"));
    }
    Ok(v)
}
fn direction(data: &Inventory, value: &Value, default: [f64; 3], e: &Entity) -> Result<V, Error> {
    match value {
        Value::Omitted => Ok(default.map(Interval::point)),
        Value::Ref(id) => {
            let d = &data.entities()[id];
            match call(d, "DIRECTION")? {
                [Value::String(_), v] => unit(vector3(d, v)?.map(Interval::point), d),
                _ => Err(fail(d, "invalid source-plane direction")),
            }
        }
        _ => Err(fail(e, "unsupported source-plane axis encoding")),
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
        let (origin, z, x) = match call(frame, "AXIS2_PLACEMENT_3D")? {
            [Value::String(_), Value::Ref(p), z, x] => (*p, z, x),
            _ => return Err(fail(frame, "invalid source-plane placement")),
        };
        let z = direction(data, z, [0., 0., 1.], frame)?;
        let reference = direction(data, x, [1., 0., 0.], frame)?;
        let y = unit(cross(z, reference), frame)?;
        let x = unit(cross(y, z), frame)?;
        // A positive orientation check avoids silently accepting an unresolved
        // interval frame when the reference direction is nearly parallel.
        if dot(cross(x, y), z).lo <= 0. {
            return Err(fail(frame, "source-plane frame orientation not certified"));
        }
        let p = &data.entities()[&origin];
        let coordinates = match call(p, "CARTESIAN_POINT")? {
            [Value::String(_), v] => vector3(p, v)?,
            _ => return Err(fail(p, "invalid source-plane origin")),
        };
        let scale_mm = context.millimetres_per_source_unit();
        let origin = coordinates.map(|x| Interval::point(x) * Interval::point(scale_mm));
        if origin.iter().any(|x| !x.finite()) {
            return Err(fail(p, "source-plane origin normalization not bounded"));
        }
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
    pub fn point_bounds(&self, parameters: [f64; 2]) -> Result<Bounds3, Error> {
        self.bounds(parameters.map(|x| [x, x]))
    }
}
