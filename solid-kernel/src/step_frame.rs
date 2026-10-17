//! Shared outward-rounded source placement used by parameter maps.
use crate::{
    analytic_step::{call, vector3},
    native_interval::Interval,
    step::{Entity, Error, ErrorKind, Inventory, Value},
};
pub(crate) type V = [Interval; 3];
fn fail(e: &Entity, s: &str) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: s.into(),
    }
}
pub(crate) fn dot(a: V, b: V) -> Interval {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V, b: V) -> V {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}
pub(crate) fn unit(v: V, e: &Entity) -> Result<V, Error> {
    let m = v
        .iter()
        .map(|x| x.lo.abs().max(x.hi.abs()))
        .fold(0., f64::max);
    if !m.is_finite() || m == 0. {
        return Err(fail(e, "zero/nonfinite source placement direction"));
    }
    let v = v.map(|x| x / Interval::point(m));
    let n = (v[0].square() + v[1].square() + v[2].square()).sqrt_nonnegative();
    if !n.finite() || n.lo <= 0. {
        return Err(fail(e, "source placement direction norm not certified"));
    }
    let v = v.map(|x| x / n);
    if v.iter().any(|x| !x.finite()) {
        return Err(fail(e, "unresolved source placement normalization"));
    }
    Ok(v)
}
pub(crate) fn direction(
    data: &Inventory,
    value: &Value,
    default: [f64; 3],
    e: &Entity,
) -> Result<V, Error> {
    match value {
        Value::Omitted => Ok(default.map(Interval::point)),
        Value::Ref(id) => {
            let d = &data.entities()[id];
            match call(d, "DIRECTION")? {
                [Value::String(_), v] => unit(vector3(d, v)?.map(Interval::point), d),
                _ => Err(fail(d, "invalid source placement direction")),
            }
        }
        _ => Err(fail(e, "unsupported source placement axis encoding")),
    }
}
#[derive(Debug)]
pub(crate) struct SourceFrame {
    pub origin: V,
    pub x: V,
    pub y: V,
    pub z: V,
}
impl SourceFrame {
    pub fn read(data: &Inventory, frame: &Entity, scale_mm: f64) -> Result<Self, Error> {
        let (origin, z, x) = match call(frame, "AXIS2_PLACEMENT_3D")? {
            [Value::String(_), Value::Ref(p), z, x] => (*p, z, x),
            _ => return Err(fail(frame, "invalid source placement placement")),
        };
        let z = direction(data, z, [0., 0., 1.], frame)?;
        let reference = direction(data, x, [1., 0., 0.], frame)?;
        let y = unit(cross(z, reference), frame)?;
        let x = unit(cross(y, z), frame)?;
        // A positive orientation check avoids silently accepting an unresolved
        // interval frame when the reference direction is nearly parallel.
        if dot(cross(x, y), z).lo <= 0. {
            return Err(fail(
                frame,
                "source placement frame orientation not certified",
            ));
        }
        let p = &data.entities()[&origin];
        let coordinates = match call(p, "CARTESIAN_POINT")? {
            [Value::String(_), v] => vector3(p, v)?,
            _ => return Err(fail(p, "invalid source placement origin")),
        };
        let origin = coordinates.map(|x| Interval::point(x) * Interval::point(scale_mm));
        if origin.iter().any(|x| !x.finite()) {
            return Err(fail(p, "source placement origin normalization not bounded"));
        }
        Ok(Self { origin, x, y, z })
    }
}
