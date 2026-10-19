//! STEP text -> [`NativeBrep`] (millimetres): the topology and analytic geometry of one `MANIFOLD_SOLID_BREP`, kept as an editable
//! B-rep for the native modelling operations (`native_brep`, `native_brep_drill`). Unlike `analytic_step::AnalyticModel` (a sampled
//! chart model for meshing), this keeps exact surfaces and curves.
//!
//! Supported: PLANE, CYLINDRICAL_SURFACE, CONICAL_SURFACE, SPHERICAL_SURFACE, TOROIDAL_SURFACE faces; LINE and CIRCLE edge curves
//! (also behind `SURFACE_CURVE` / `SEAM_CURVE`); the unit and body binding of `step_body`. Anything else is an explicit error.
use crate::analytic_step::{call, curve_geometry, point as read_point, scalar, vector3};
use crate::native_brep::{Curve, Edge, Face, NativeBrep, Surface, Use};
use crate::step::{inventory, Entity, Error, Inventory, Limits, Value};
use crate::step_body::BodyContext;
use crate::V3;
use std::collections::BTreeMap;

fn e2s(e: Error) -> String {
    e.to_string()
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn unit(a: V3) -> Result<V3, String> {
    let l = dot(a, a).sqrt();
    if l > 0.0 && l.is_finite() {
        Ok([a[0] / l, a[1] / l, a[2] / l])
    } else {
        Err("a zero direction".into())
    }
}

fn entity<'a>(data: &'a Inventory, id: u64) -> Result<&'a Entity, String> {
    data.entities().get(&id).ok_or_else(|| format!("missing entity #{id}"))
}
fn direction(data: &Inventory, id: u64) -> Result<V3, String> {
    let e = entity(data, id)?;
    match call(e, "DIRECTION").map_err(e2s)? {
        [Value::String(_), v] => unit(vector3(e, v).map_err(e2s)?),
        _ => Err(format!("#{id}: unsupported direction")),
    }
}
/// (origin, axis, x direction) of an `AXIS2_PLACEMENT_3D`, lengths in millimetres.
fn placement(data: &Inventory, id: u64, scale: f64) -> Result<(V3, V3, V3), String> {
    let e = entity(data, id)?;
    match call(e, "AXIS2_PLACEMENT_3D").map_err(e2s)? {
        [Value::String(_), Value::Ref(p), axis, reference] => {
            let origin = read_point(data, *p, scale).map_err(e2s)?;
            let z = match axis {
                Value::Ref(a) => direction(data, *a)?,
                _ => [0., 0., 1.],
            };
            let r = match reference {
                Value::Ref(a) => direction(data, *a)?,
                _ => [1., 0., 0.],
            };
            // The reference direction is made perpendicular to the axis.
            let d = dot(r, z);
            let x = unit(sub(r, [z[0] * d, z[1] * d, z[2] * d])).map_err(|_| format!("#{id}: reference direction parallel to the axis"))?;
            Ok((origin, z, x))
        }
        _ => Err(format!("#{id}: unsupported placement")),
    }
}
fn number(data: &Inventory, e: &Entity, v: &Value, scale: f64) -> Result<f64, String> {
    let _ = data;
    Ok(scalar(e, v).map_err(e2s)? * scale)
}

fn read_surface(data: &Inventory, id: u64, scale: f64) -> Result<Surface, String> {
    let e = entity(data, id)?;
    let c = e.components.first().ok_or("empty surface entity")?;
    if e.components.len() != 1 {
        return Err(format!("#{id}: unsupported complex surface"));
    }
    let args = &c.args;
    let place = |v: &Value| match v {
        Value::Ref(p) => placement(data, *p, scale),
        _ => Err(format!("#{id}: unsupported surface placement")),
    };
    match (c.name.as_str(), args.as_slice()) {
        ("PLANE", [_, p]) => {
            let (origin, normal, xdir) = place(p)?;
            Ok(Surface::Plane { origin, normal, xdir })
        }
        ("CYLINDRICAL_SURFACE", [_, p, r]) => {
            let (origin, axis, xdir) = place(p)?;
            Ok(Surface::Cylinder { origin, axis, xdir, radius: number(data, e, r, scale)? })
        }
        ("CONICAL_SURFACE", [_, p, r, a]) => {
            let (origin, axis, xdir) = place(p)?;
            Ok(Surface::Cone { origin, axis, xdir, radius: number(data, e, r, scale)?, semi_angle: number(data, e, a, 1.0)? })
        }
        ("SPHERICAL_SURFACE", [_, p, r]) => {
            let (centre, axis, xdir) = place(p)?;
            Ok(Surface::Sphere { centre, axis, xdir, radius: number(data, e, r, scale)? })
        }
        ("TOROIDAL_SURFACE", [_, p, a, b]) => {
            let (centre, axis, xdir) = place(p)?;
            Ok(Surface::Torus { centre, axis, xdir, major: number(data, e, a, scale)?, minor: number(data, e, b, scale)? })
        }
        (other, _) => Err(format!("#{id}: the surface type {other} is not supported by the native modeller")),
    }
}

fn read_curve(data: &Inventory, id: u64, scale: f64) -> Result<Curve, String> {
    let e = curve_geometry(data, id).map_err(e2s)?;
    let c = e.components.first().ok_or("empty curve entity")?;
    if e.components.len() != 1 {
        return Err(format!("#{}: unsupported complex curve", e.id));
    }
    match (c.name.as_str(), c.args.as_slice()) {
        ("LINE", [_, Value::Ref(p), Value::Ref(v)]) => {
            let origin = read_point(data, *p, scale).map_err(e2s)?;
            let ve = entity(data, *v)?;
            let dir = match call(ve, "VECTOR").map_err(e2s)? {
                [Value::String(_), Value::Ref(d), _] => direction(data, *d)?,
                _ => return Err(format!("#{v}: unsupported vector")),
            };
            Ok(Curve::Line { origin, direction: dir })
        }
        ("CIRCLE", [_, Value::Ref(p), r]) => {
            let (centre, axis, xdir) = placement(data, *p, scale)?;
            Ok(Curve::Circle { centre, axis, xdir, radius: number(data, e, r, scale)? })
        }
        (other, _) => Err(format!("#{}: the edge curve type {other} is not supported by the native modeller", e.id)),
    }
}

/// A rough size of a loop (the extent of its vertices) to tell the outer bound from holes when the file does not say.
fn loop_extent(b: &NativeBrep, uses: &[Use]) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for u in uses {
        let e = &b.edges[u.edge];
        let mut pts = vec![b.vertices[e.start], b.vertices[e.end]];
        if let Curve::Circle { centre, radius, .. } = &e.curve {
            pts.push([centre[0] + radius, centre[1] + radius, centre[2] + radius]);
            pts.push([centre[0] - radius, centre[1] - radius, centre[2] - radius]);
        }
        for p in pts {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max)
}

impl NativeBrep {
    /// Read the single `MANIFOLD_SOLID_BREP` of a STEP text.
    pub fn from_step_text(text: &str) -> Result<Self, String> {
        let data = inventory(text, Limits::default()).map_err(e2s)?;
        let ctx = BodyContext::read(&data).map_err(e2s)?;
        let scale = ctx.millimetres_per_source_unit();
        let body = ctx.body();
        let mut out = NativeBrep::default();
        let mut vmap = BTreeMap::new();
        for (id, v) in &body.vertices {
            vmap.insert(*id, out.vertices.len());
            out.vertices.push(read_point(&data, v.point, scale).map_err(e2s)?);
        }
        let mut emap = BTreeMap::new();
        let mut reversed = BTreeMap::new();
        for (id, e) in &body.edges {
            let curve = read_curve(&data, e.curve, scale)?;
            let (a, b) = (vmap[&e.start], vmap[&e.end]);
            // `same_sense: false` means the curve runs from the edge's end to its start: store the edge along its curve.
            let (start, end) = if e.same_sense { (a, b) } else { (b, a) };
            emap.insert(*id, out.edges.len());
            reversed.insert(*id, !e.same_sense);
            out.edges.push(Edge { curve, start, end });
        }
        let mut surfaces = BTreeMap::new();
        for f in body.faces.values() {
            if !surfaces.contains_key(&f.surface) {
                surfaces.insert(f.surface, read_surface(&data, f.surface, scale)?);
            }
        }
        for f in body.faces.values() {
            let mut loops: Vec<Vec<Use>> = Vec::new();
            for bound in &f.bounds {
                let uses: Vec<Use> = bound
                    .edges
                    .iter()
                    .map(|u| Use { edge: emap[&u.edge], forward: u.forward != reversed[&u.edge] })
                    .collect();
                // A bound used against its declared orientation runs backwards.
                let uses = if bound.forward { uses } else { uses.into_iter().rev().map(|u| Use { edge: u.edge, forward: !u.forward }).collect() };
                loops.push(uses);
            }
            if loops.is_empty() {
                return Err(format!("face #{} has no bound", f.id));
            }
            let outer_at = if loops.len() == 1 {
                0
            } else {
                let explicit: Vec<usize> = f.bounds.iter().enumerate().filter(|(_, b)| b.explicitly_outer).map(|(i, _)| i).collect();
                match explicit.as_slice() {
                    [i] => *i,
                    _ => {
                        // The largest loop is the outer one; a face whose loops tie in size (a ring of two circles is
                        // distinguished by radius, so ties are unusual) is refused.
                        let ext: Vec<f64> = loops.iter().map(|l| loop_extent(&out, l)).collect();
                        let mut order: Vec<usize> = (0..ext.len()).collect();
                        order.sort_by(|a, b| ext[*b].total_cmp(&ext[*a]));
                        let (best, second) = (order[0], ext[order[1]]);
                        if (ext[best] - second).abs() <= 1e-9 * ext[best].max(1.0) {
                            return Err(format!("face #{}: cannot tell its outer bound from its holes", f.id));
                        }
                        best
                    }
                }
            };
            let outer = loops.remove(outer_at);
            out.faces.push(Face { surface: surfaces[&f.surface].clone(), same_sense: f.same_sense, outer, holes: loops });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_native_text_reads_back_to_the_same_shape() {
        let b = NativeBrep::box_from_corners([0., 0., 0.], [2., 3., 4.]).unwrap();
        let back = NativeBrep::from_step_text(&b.to_step_text()).unwrap();
        assert_eq!((back.vertices.len(), back.edges.len(), back.faces.len()), (b.vertices.len(), b.edges.len(), b.faces.len()));
        // Vertices are read in entity-id order, which is the order they were written.
        for (a, c) in back.vertices.iter().zip(&b.vertices) {
            assert_eq!(a, c);
        }
        assert_eq!(back.faces[0].surface, b.faces[0].surface);
    }
}
