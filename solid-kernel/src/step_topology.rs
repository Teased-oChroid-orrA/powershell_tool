//! Read-only topology extraction for simple MANIFOLD_SOLID_BREP declarations.
//! Connectivity/edge-incidence checks do not prove geometric validity, orientability,
//! vertex-manifoldness, trimming, units/placement mapping or a production solid.
use crate::step::{Call, Entity, Error, ErrorKind, Inventory, Value};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone)]
pub struct RawVertex {
    pub id: u64,
    pub point: u64,
    pub coordinates: [f64; 3],
}
#[derive(Debug, Clone)]
pub struct RawEdge {
    pub id: u64,
    pub start: u64,
    pub end: u64,
    pub curve: u64,
    pub same_sense: bool,
}
#[derive(Debug, Clone)]
pub struct EdgeUse {
    pub id: u64,
    pub edge: u64,
    pub forward: bool,
}
#[derive(Debug, Clone)]
pub struct RawBound {
    pub id: u64,
    pub loop_id: u64,
    pub forward: bool,
    /// FACE_BOUND does not identify an outer loop; do not infer one from ordering.
    pub explicitly_outer: bool,
    pub edges: Vec<EdgeUse>,
}
#[derive(Debug, Clone)]
pub struct RawFace {
    pub id: u64,
    pub label: String,
    pub surface: u64,
    pub same_sense: bool,
    pub bounds: Vec<RawBound>,
}
#[derive(Debug, Clone)]
pub struct RawBody {
    pub id: u64,
    pub label: String,
    pub shell: u64,
    pub faces: BTreeMap<u64, RawFace>,
    pub edges: BTreeMap<u64, RawEdge>,
    pub vertices: BTreeMap<u64, RawVertex>,
}
/// Cumulative expansion limits across all returned bodies, not per-body limits.
/// Shared source shells can otherwise expand a small file into unbounded copies.
#[derive(Debug, Clone, Copy)]
pub struct TopologyLimits {
    pub bodies: usize,
    pub faces: usize,
    pub edges: usize,
    pub vertices: usize,
    pub edge_uses: usize,
    pub label_bytes: usize,
}
impl Default for TopologyLimits {
    fn default() -> Self {
        Self {
            bodies: 1024,
            faces: 100_000,
            edges: 200_000,
            vertices: 200_000,
            edge_uses: 400_000,
            label_bytes: 8 * 1024 * 1024,
        }
    }
}
#[derive(Default)]
struct Used {
    bodies: usize,
    faces: usize,
    edges: usize,
    vertices: usize,
    edge_uses: usize,
    label_bytes: usize,
}
fn charge(e: &Entity, used: &mut usize, amount: usize, limit: usize) -> Result<(), Error> {
    *used = used
        .checked_add(amount)
        .filter(|n| *n <= limit)
        .ok_or_else(|| Error {
            offset: e.offset,
            kind: ErrorKind::Limit,
            detail: "cumulative topology expansion budget exceeded".into(),
        })?;
    Ok(())
}
fn error(e: &Entity, s: impl Into<String>) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Topology,
        detail: s.into(),
    }
}
fn simple<'a>(e: &'a Entity, name: &str) -> Result<&'a Call, Error> {
    match e.components.as_slice() {
        [c] if c.name == name => Ok(c),
        _ => Err(error(
            e,
            format!("unsupported {name} entity encoding for #{}", e.id),
        )),
    }
}
fn boolean(e: &Entity, v: &Value) -> Result<bool, Error> {
    match v {
        Value::Enum(s) if s == "T" => Ok(true),
        Value::Enum(s) if s == "F" => Ok(false),
        _ => Err(error(e, "expected an explicit boolean orientation")),
    }
}
fn reference(e: &Entity, v: &Value) -> Result<u64, Error> {
    match v {
        Value::Ref(id) => Ok(*id),
        _ => Err(error(e, "expected entity reference")),
    }
}
fn refs(e: &Entity, v: &Value) -> Result<Vec<u64>, Error> {
    match v {
        Value::List(values) => values.iter().map(|v| reference(e, v)).collect(),
        _ => Err(error(e, "expected reference list")),
    }
}
fn real(e: &Entity, v: &Value) -> Result<f64, Error> {
    match v {
        Value::Real(x) if x.is_finite() => Ok(*x),
        Value::Integer(x) if x.unsigned_abs() <= 1u64 << 53 => Ok(*x as f64),
        _ => Err(error(e, "coordinate is not supported exactly as f64")),
    }
}
fn vertex(data: &Inventory, id: u64) -> Result<RawVertex, Error> {
    let e = &data.entities()[&id];
    let c = simple(e, "VERTEX_POINT")?;
    let p = match c.args.as_slice() {
        [Value::String(_), Value::Ref(p)] => *p,
        _ => return Err(error(e, "unsupported vertex declaration")),
    };
    let pe = &data.entities()[&p];
    let pc = simple(pe, "CARTESIAN_POINT")?;
    let coordinates = match pc.args.as_slice() {
        [Value::String(_), Value::List(v)] if v.len() == 3 => {
            [real(pe, &v[0])?, real(pe, &v[1])?, real(pe, &v[2])?]
        }
        _ => return Err(error(pe, "solid vertex requires an explicit 3D point")),
    };
    Ok(RawVertex {
        id,
        point: p,
        coordinates,
    })
}
fn edge(data: &Inventory, id: u64) -> Result<RawEdge, Error> {
    let e = &data.entities()[&id];
    let c = simple(e, "EDGE_CURVE")?;
    match c.args.as_slice() {
        [Value::String(_), Value::Ref(a), Value::Ref(b), Value::Ref(curve), sense] => Ok(RawEdge {
            id,
            start: *a,
            end: *b,
            curve: *curve,
            same_sense: boolean(e, sense)?,
        }),
        _ => Err(error(e, "unsupported edge declaration")),
    }
}
/// Whether the face bound `id` is made of a `VERTEX_LOOP` (a single point, no edge).
fn is_vertex_loop_bound(data: &Inventory, id: u64) -> bool {
    let Some(e) = data.entities().get(&id) else {
        return false;
    };
    let [c] = e.components.as_slice() else {
        return false;
    };
    if c.name != "FACE_BOUND" && c.name != "FACE_OUTER_BOUND" {
        return false;
    }
    let [Value::String(_), Value::Ref(loop_id), _] = c.args.as_slice() else {
        return false;
    };
    data.entities()
        .get(loop_id)
        .is_some_and(|l| matches!(l.components.as_slice(), [c] if c.name == "VERTEX_LOOP"))
}
fn bound(
    data: &Inventory,
    id: u64,
    body: &mut RawBody,
    used: &mut Used,
    limits: TopologyLimits,
) -> Result<RawBound, Error> {
    let e = &data.entities()[&id];
    let (c, outer) = match e.components.as_slice() {
        [c] if c.name == "FACE_BOUND" => (c, false),
        [c] if c.name == "FACE_OUTER_BOUND" => (c, true),
        _ => return Err(error(e, "unsupported face bound")),
    };
    let (loop_id, forward) = match c.args.as_slice() {
        [Value::String(_), Value::Ref(id), forward] => (*id, boolean(e, forward)?),
        _ => return Err(error(e, "invalid face bound")),
    };
    let le = &data.entities()[&loop_id];
    let lc = simple(le, "EDGE_LOOP")?;
    let oriented = match lc.args.as_slice() {
        [Value::String(_), v] => refs(le, v)?,
        _ => return Err(error(le, "invalid edge loop")),
    };
    if oriented.is_empty() {
        return Err(error(le, "empty edge loop is not supported"));
    }
    let mut uses = Vec::new();
    let mut chain = Vec::new();
    let mut unique = BTreeSet::new();
    for id in oriented {
        charge(le, &mut used.edge_uses, 1, limits.edge_uses)?;
        if !unique.insert(id) {
            return Err(error(le, "repeated oriented-edge identity in one loop"));
        }
        let oe = &data.entities()[&id];
        let oc = simple(oe, "ORIENTED_EDGE")?;
        let (base, forward) = match oc.args.as_slice() {
            [Value::String(_), Value::Derived, Value::Derived, Value::Ref(base), sense] => {
                (*base, boolean(oe, sense)?)
            }
            _ => return Err(error(oe, "unsupported oriented edge")),
        };
        if !body.edges.contains_key(&base) {
            charge(oe, &mut used.edges, 1, limits.edges)?;
            let edge = edge(data, base)?;
            for v in [edge.start, edge.end] {
                if !body.vertices.contains_key(&v) {
                    charge(oe, &mut used.vertices, 1, limits.vertices)?;
                    body.vertices.insert(v, vertex(data, v)?);
                }
            }
            body.edges.insert(base, edge);
        }
        let edge = &body.edges[&base];
        chain.push(if forward {
            (edge.start, edge.end)
        } else {
            (edge.end, edge.start)
        });
        uses.push(EdgeUse {
            id,
            edge: base,
            forward,
        });
    }
    for i in 0..chain.len() {
        if chain[i].1 != chain[(i + 1) % chain.len()].0 {
            return Err(error(
                le,
                "edge loop is not closed by vertex identity; healing is not inferred",
            ));
        }
    }
    Ok(RawBound {
        id,
        loop_id,
        forward,
        explicitly_outer: outer,
        edges: uses,
    })
}
/// Extract every declared simple manifold body. Bodies are in raw local coordinates;
/// geometry IDs, bound orientation and unspecified outer-loop roles remain explicit.
/// Assembly placements and representation/unit associations are not applied.
pub fn bodies(data: &Inventory, limits: TopologyLimits) -> Result<Vec<RawBody>, Error> {
    let mut used = Used::default();
    let mut out = Vec::new();
    for e in data.entities().values() {
        if e.components.iter().any(|c| {
            matches!(
                c.name.as_str(),
                "BREP_WITH_VOIDS" | "FACETED_BREP" | "CSG_SOLID" | "SOLID_REPLICA"
            )
        }) {
            return Err(error(
                e,
                "unsupported body kind; no solid may be silently omitted",
            ));
        }
        if !e.components.iter().any(|c| c.name == "MANIFOLD_SOLID_BREP") {
            continue;
        }
        charge(e, &mut used.bodies, 1, limits.bodies)?;
        let c = simple(e, "MANIFOLD_SOLID_BREP")?;
        let (label, shell) = match c.args.as_slice() {
            [Value::String(label), Value::Ref(shell)] => (label, *shell),
            _ => return Err(error(e, "unsupported manifold solid declaration")),
        };
        charge(e, &mut used.label_bytes, label.len(), limits.label_bytes)?;
        let label = label.clone();
        let se = &data.entities()[&shell];
        let sc = simple(se, "CLOSED_SHELL")?;
        let face_ids = match sc.args.as_slice() {
            [Value::String(_), v] => refs(se, v)?,
            _ => return Err(error(se, "unsupported closed shell")),
        };
        if face_ids.is_empty() {
            return Err(error(se, "empty shell"));
        }
        let mut body = RawBody {
            id: e.id,
            label,
            shell,
            faces: BTreeMap::new(),
            edges: BTreeMap::new(),
            vertices: BTreeMap::new(),
        };
        let mut owners = BTreeMap::<u64, Vec<u64>>::new();
        for id in face_ids {
            charge(se, &mut used.faces, 1, limits.faces)?;
            if body.faces.contains_key(&id) {
                return Err(error(se, "repeated face identity in shell"));
            }
            let fe = &data.entities()[&id];
            let fc = simple(fe, "ADVANCED_FACE")?;
            let (label, bounds, surface, same_sense) = match fc.args.as_slice() {
                [Value::String(label), bounds, Value::Ref(surface), sense] => {
                    (label, refs(fe, bounds)?, *surface, boolean(fe, sense)?)
                }
                _ => return Err(error(fe, "unsupported advanced face")),
            };
            charge(fe, &mut used.label_bytes, label.len(), limits.label_bytes)?;
            let label = label.clone();
            if bounds.is_empty() {
                return Err(error(fe, "unbounded/whole-surface face is not supported"));
            }
            let mut parsed = Vec::new();
            let mut unique = BTreeSet::new();
            // Parasolid/Onshape write the pole of a sphere or the apex of a cone as an extra bound made of one vertex. The
            // native chart of those surfaces is local to the face (the source parameterization's pole plays no role), so
            // such a bound carries no boundary information and is skipped; on any other surface it stays unsupported.
            let pole_surface = data.entities().get(&surface).is_some_and(|s| {
                matches!(
                    s.components.as_slice(),
                    [c] if c.name == "SPHERICAL_SURFACE" || c.name == "CONICAL_SURFACE"
                )
            });
            for id in bounds {
                if !unique.insert(id) {
                    return Err(error(fe, "duplicate face bound"));
                }
                if pole_surface && is_vertex_loop_bound(data, id) {
                    continue;
                }
                let b = bound(data, id, &mut body, &mut used, limits)?;
                for u in &b.edges {
                    owners.entry(u.edge).or_default().push(fe.id);
                }
                parsed.push(b);
            }
            // Onshape (the ABC dataset) writes hole loops as FACE_OUTER_BOUND too, so several declared outers are data, not an
            // error: the roles are decided geometrically (`planar_step::classify`, `cylindrical_step`), which rejects a face
            // that still has no single enclosing loop.
            body.faces.insert(
                id,
                RawFace {
                    id,
                    label,
                    surface,
                    same_sense,
                    bounds: parsed,
                },
            );
        }
        for (edge, faces) in &owners {
            if faces.len() != 2 {
                return Err(error(
                    &data.entities()[edge],
                    format!("closed-shell edge has {} uses instead of two", faces.len()),
                ));
            }
        }
        let mut adjacent = BTreeMap::<u64, Vec<u64>>::new();
        for faces in owners.values() {
            adjacent.entry(faces[0]).or_default().push(faces[1]);
            adjacent.entry(faces[1]).or_default().push(faces[0]);
        }
        let mut seen = BTreeSet::new();
        let mut pending = vec![*body.faces.keys().next().unwrap()];
        while let Some(face) = pending.pop() {
            if seen.insert(face) {
                if let Some(next) = adjacent.get(&face) {
                    pending.extend(next);
                }
            }
        }
        if seen.len() != body.faces.len() {
            return Err(error(se, "closed shell has disconnected face components"));
        }
        out.push(body);
    }
    if out.is_empty() {
        return Err(Error {
            offset: 0,
            kind: ErrorKind::Topology,
            detail: "no supported manifold body declarations".into(),
        });
    }
    Ok(out)
}
