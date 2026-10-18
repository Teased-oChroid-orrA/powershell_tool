//! STEP product structure: placed instances of the solid parts of an assembly (epic #51, "assemblies and placements").
//!
//! `step_body::BodyContext` refuses assemblies. This module reads the explicit product structure
//! (`NEXT_ASSEMBLY_USAGE_OCCURRENCE`, `PRODUCT_DEFINITION_SHAPE`, `SHAPE_DEFINITION_REPRESENTATION`,
//! `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` with `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION` and
//! `ITEM_DEFINED_TRANSFORMATION`) of a tree-shaped assembly and returns, for every leaf instance, the part's solid and the rigid
//! placement that carries its coordinates into the root's. The direction of a transformation is a known ambiguity of the format
//! (the order of the two representations and of the two placements varies between writers). The rule here was fixed against an
//! independent reference, OpenCASCADE reading the same files (`tests/step_assembly.rs`, feature `occt`): the placement
//! that is an item of the child's representation is the *source* frame, the one that is an item of the parent's representation
//! the *target* frame, whatever order they are written in, and `T = Target * Source^-1`. A placement that belongs to neither or
//! to both representations, differing length units between parent and child, non-rigid (non-orthonormal) frames, mapped items,
//! cycles, a part without exactly one solid, and more than one root are refused, never guessed.

use crate::step::{Call, Entity, Error, ErrorKind, Inventory, RawBody, Value};
use std::collections::{BTreeMap, BTreeSet};

/// A rigid motion `x' = R x + t` (rows of `rotation` are the rows of R), in the source length unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub rotation: [[f64; 3]; 3],
    pub translation: [f64; 3],
}

impl Placement {
    pub const IDENTITY: Placement = Placement { rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], translation: [0.; 3] };

    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| (0..3).map(|j| self.rotation[i][j] * p[j]).sum::<f64>() + self.translation[i])
    }

    /// `self` after `inner`: `x -> self(inner(x))`.
    pub fn after(&self, inner: &Placement) -> Placement {
        let rotation = std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| self.rotation[i][k] * inner.rotation[k][j]).sum()));
        Placement { rotation, translation: self.apply(inner.translation) }
    }

    pub fn inverse(&self) -> Placement {
        let rotation = std::array::from_fn(|i| std::array::from_fn(|j| self.rotation[j][i]));
        let inv = Placement { rotation, translation: [0.; 3] };
        let t = inv.apply(self.translation);
        Placement { rotation, translation: [-t[0], -t[1], -t[2]] }
    }
}

/// One leaf instance of the assembly.
#[derive(Debug, Clone)]
pub struct PlacedPart {
    /// The part's `MANIFOLD_SOLID_BREP`.
    pub solid: u64,
    /// The part's `PRODUCT_DEFINITION`.
    pub definition: u64,
    /// The `NEXT_ASSEMBLY_USAGE_OCCURRENCE` ids from the root down to this instance.
    pub path: Vec<u64>,
    /// Carries the part's coordinates into the root's frame.
    pub placement: Placement,
}

#[derive(Debug, Clone)]
pub struct Assembly {
    /// The root `PRODUCT_DEFINITION`.
    pub root: u64,
    pub parts: Vec<PlacedPart>,
}

fn fail(detail: impl Into<String>) -> Error {
    Error { offset: 0, kind: ErrorKind::Geometry, detail: detail.into() }
}

fn call<'a>(e: &'a Entity, name: &str) -> Option<&'a Call> {
    e.components.iter().find(|c| c.name == name)
}

fn reference(v: &Value) -> Option<u64> {
    if let Value::Ref(r) = v {
        Some(*r)
    } else {
        None
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Real(x) => Some(*x),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

fn vector(data: &Inventory, id: u64, name: &str) -> Result<[f64; 3], Error> {
    let e = data.entities().get(&id).ok_or_else(|| fail(format!("missing #{id}")))?;
    let c = call(e, name).ok_or_else(|| fail(format!("#{id} is not a {name}")))?;
    match c.args.as_slice() {
        [_, Value::List(v)] if v.len() == 3 => {
            let x: Option<Vec<f64>> = v.iter().map(number).collect();
            let x = x.ok_or_else(|| fail(format!("#{id}: non-numeric coordinate")))?;
            if x.iter().any(|v| !v.is_finite()) {
                return Err(fail(format!("#{id}: non-finite coordinate")));
            }
            Ok([x[0], x[1], x[2]])
        }
        _ => Err(fail(format!("#{id}: a {name} needs three coordinates"))),
    }
}

/// The frame of an `AXIS2_PLACEMENT_3D` as a placement (local axes as the columns of the rotation); orthonormal or refused.
fn frame(data: &Inventory, id: u64) -> Result<Placement, Error> {
    let e = data.entities().get(&id).ok_or_else(|| fail(format!("missing placement #{id}")))?;
    let c = call(e, "AXIS2_PLACEMENT_3D").ok_or_else(|| fail(format!("#{id} is not an AXIS2_PLACEMENT_3D")))?;
    let [_, loc, axis, refd] = c.args.as_slice() else {
        return Err(fail(format!("#{id}: an AXIS2_PLACEMENT_3D needs four arguments")));
    };
    let origin = vector(data, reference(loc).ok_or_else(|| fail(format!("#{id}: no location")))?, "CARTESIAN_POINT")?;
    let (axis, refd) = match (axis, refd) {
        (Value::Ref(a), Value::Ref(r)) => (vector(data, *a, "DIRECTION")?, vector(data, *r, "DIRECTION")?),
        (Value::Omitted, Value::Omitted) => ([0., 0., 1.], [1., 0., 0.]),
        _ => return Err(fail(format!("#{id}: axis and reference direction must both be given or both omitted"))),
    };
    let norm = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let n = norm(axis);
    if n < 1e-12 || norm(refd) < 1e-12 {
        return Err(fail(format!("#{id}: zero direction")));
    }
    let z = [axis[0] / n, axis[1] / n, axis[2] / n];
    let along = refd[0] * z[0] + refd[1] * z[1] + refd[2] * z[2];
    let x0 = [refd[0] - along * z[0], refd[1] - along * z[1], refd[2] - along * z[2]];
    let nx = norm(x0);
    if nx < 1e-9 * norm(refd) {
        return Err(fail(format!("#{id}: the reference direction is parallel to the axis")));
    }
    let x = [x0[0] / nx, x0[1] / nx, x0[2] / nx];
    let y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
    // The STEP definition is a right-handed frame from (axis, reference): the columns are the local x, y, z axes.
    Ok(Placement { rotation: [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]], translation: origin })
}

struct Product<'a> {
    data: &'a Inventory,
    /// definition -> its shape representation (through PRODUCT_DEFINITION_SHAPE and SHAPE_DEFINITION_REPRESENTATION)
    rep_of: BTreeMap<u64, u64>,
    /// parent definition -> (nauo, child definition)
    children: BTreeMap<u64, Vec<(u64, u64)>>,
    /// nauo -> (item placement ids of the two representations, rep_1, rep_2, transformation items)
    transform_of: BTreeMap<u64, (u64, u64, u64, u64)>,
    bodies: BTreeMap<u64, RawBody>,
}

impl Product<'_> {
    fn items(&self, rep: u64) -> Result<Vec<u64>, Error> {
        let e = self.data.entities().get(&rep).ok_or_else(|| fail(format!("missing representation #{rep}")))?;
        for c in &e.components {
            if matches!(c.name.as_str(), "SHAPE_REPRESENTATION" | "ADVANCED_BREP_SHAPE_REPRESENTATION") {
                if let [_, Value::List(items), _] = c.args.as_slice() {
                    return Ok(items.iter().filter_map(reference).collect());
                }
            }
        }
        Err(fail(format!("#{rep} is not a supported shape representation")))
    }

    fn context(&self, rep: u64) -> Result<u64, Error> {
        let e = &self.data.entities()[&rep];
        for c in &e.components {
            if let [_, Value::List(_), Value::Ref(ctx)] = c.args.as_slice() {
                return Ok(*ctx);
            }
        }
        Err(fail(format!("#{rep} has no representation context")))
    }

    fn metres(&self, rep: u64) -> Result<f64, Error> {
        let ctx = self.context(rep)?;
        self.data.length_contexts()?.iter().find(|c| c.context == ctx).map(|c| c.metres).ok_or_else(|| fail(format!("representation #{rep} has no assigned length unit")))
    }

    fn walk(&self, definition: u64, world: Placement, path: &mut Vec<u64>, out: &mut Vec<PlacedPart>, active: &mut BTreeSet<u64>) -> Result<(), Error> {
        if !active.insert(definition) {
            return Err(fail(format!("product definition #{definition} contains itself")));
        }
        match self.children.get(&definition) {
            None => {
                let rep = *self.rep_of.get(&definition).ok_or_else(|| fail(format!("leaf definition #{definition} has no shape representation")))?;
                let solids: Vec<u64> = self.items(rep)?.into_iter().filter(|i| self.bodies.contains_key(i)).collect();
                match solids.as_slice() {
                    [solid] => out.push(PlacedPart { solid: *solid, definition, path: path.clone(), placement: world }),
                    other => return Err(fail(format!("part #{definition} has {} solids; exactly one is supported", other.len()))),
                }
            }
            Some(children) => {
                let parent_rep = *self.rep_of.get(&definition).ok_or_else(|| fail(format!("assembly #{definition} has no shape representation")))?;
                for &(nauo, child) in children {
                    let child_rep = *self.rep_of.get(&child).ok_or_else(|| fail(format!("component #{child} has no shape representation")))?;
                    let (rep1, rep2, item1, item2) = *self.transform_of.get(&nauo).ok_or_else(|| fail(format!("usage #{nauo} has no placement transformation")))?;
                    if !((rep1 == child_rep && rep2 == parent_rep) || (rep1 == parent_rep && rep2 == child_rep)) {
                        return Err(fail(format!("usage #{nauo}: the relationship does not join the component's and the assembly's representations")));
                    }
                    let (child_items, parent_items) = (self.items(child_rep)?, self.items(parent_rep)?);
                    let in_child = |i: u64| child_items.contains(&i);
                    let in_parent = |i: u64| parent_items.contains(&i);
                    let (source, target) = if in_child(item1) && in_parent(item2) && !in_child(item2) && !in_parent(item1) {
                        (item1, item2)
                    } else if in_child(item2) && in_parent(item1) && !in_child(item1) && !in_parent(item2) {
                        (item2, item1)
                    } else {
                        return Err(fail(format!("usage #{nauo}: cannot tell which placement belongs to the component and which to the assembly")));
                    };
                    if (self.metres(child_rep)? - self.metres(parent_rep)?).abs() > 1e-12 * self.metres(parent_rep)?.abs() {
                        return Err(fail(format!("usage #{nauo}: component and assembly use different length units")));
                    }
                    let t = frame(self.data, target)?.after(&frame(self.data, source)?.inverse());
                    path.push(nauo);
                    self.walk(child, world.after(&t), path, out, active)?;
                    path.pop();
                }
            }
        }
        active.remove(&definition);
        Ok(())
    }
}

/// Read the instances of a tree-shaped assembly. A file without product structure is refused (use `BodyContext` for a single
/// body); so is anything the module header lists.
pub fn read(data: &Inventory) -> Result<Assembly, Error> {
    let ents = data.entities();
    if ents.values().any(|e| e.components.iter().any(|c| c.name == "MAPPED_ITEM")) {
        return Err(fail("mapped items (representation maps) are not implemented"));
    }
    let mut pds_definition: BTreeMap<u64, u64> = BTreeMap::new();
    let mut nauo_ids: BTreeSet<u64> = BTreeSet::new();
    let mut children: BTreeMap<u64, Vec<(u64, u64)>> = BTreeMap::new();
    let mut related: BTreeSet<u64> = BTreeSet::new();
    for (id, e) in ents {
        if let Some(c) = call(e, "PRODUCT_DEFINITION_SHAPE") {
            if let [_, _, Value::Ref(def)] = c.args.as_slice() {
                pds_definition.insert(*id, *def);
            }
        }
        if let Some(c) = call(e, "NEXT_ASSEMBLY_USAGE_OCCURRENCE") {
            let [_, _, _, Value::Ref(parent), Value::Ref(child), ..] = c.args.as_slice() else {
                return Err(fail(format!("#{id}: malformed NEXT_ASSEMBLY_USAGE_OCCURRENCE")));
            };
            nauo_ids.insert(*id);
            children.entry(*parent).or_default().push((*id, *child));
            related.insert(*child);
        }
    }
    if nauo_ids.is_empty() {
        return Err(fail("no NEXT_ASSEMBLY_USAGE_OCCURRENCE: not an assembly"));
    }
    let mut rep_of: BTreeMap<u64, u64> = BTreeMap::new();
    for e in ents.values() {
        if let Some(c) = call(e, "SHAPE_DEFINITION_REPRESENTATION") {
            if let [Value::Ref(pds), Value::Ref(rep)] = c.args.as_slice() {
                if let Some(def) = pds_definition.get(pds) {
                    if rep_of.insert(*def, *rep).is_some() {
                        return Err(fail(format!("definition #{def} has several shape representations")));
                    }
                }
            }
        }
    }
    let mut transform_of: BTreeMap<u64, (u64, u64, u64, u64)> = BTreeMap::new();
    for e in ents.values() {
        let Some(c) = call(e, "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION") else { continue };
        let [Value::Ref(relationship), Value::Ref(pds)] = c.args.as_slice() else { continue };
        let Some(nauo) = pds_definition.get(pds).filter(|d| nauo_ids.contains(d)) else { continue };
        let rel = ents.get(relationship).ok_or_else(|| fail(format!("missing relationship #{relationship}")))?;
        let reps = call(rel, "REPRESENTATION_RELATIONSHIP").ok_or_else(|| fail(format!("#{relationship} is not a REPRESENTATION_RELATIONSHIP")))?;
        let [_, _, Value::Ref(r1), Value::Ref(r2)] = reps.args.as_slice() else { return Err(fail(format!("#{relationship}: malformed REPRESENTATION_RELATIONSHIP"))) };
        let with = call(rel, "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION").ok_or_else(|| fail(format!("#{relationship}: no transformation (REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION)")))?;
        let Some(Value::Ref(op)) = with.args.first() else { return Err(fail(format!("#{relationship}: the transformation operator must be an ITEM_DEFINED_TRANSFORMATION reference"))) };
        let itd = ents.get(op).and_then(|e| call(e, "ITEM_DEFINED_TRANSFORMATION")).ok_or_else(|| fail(format!("#{op} is not an ITEM_DEFINED_TRANSFORMATION")))?;
        let [_, _, Value::Ref(i1), Value::Ref(i2)] = itd.args.as_slice() else { return Err(fail(format!("#{op}: malformed ITEM_DEFINED_TRANSFORMATION"))) };
        if transform_of.insert(*nauo, (*r1, *r2, *i1, *i2)).is_some() {
            return Err(fail(format!("usage #{nauo} has several transformations")));
        }
    }
    let roots: Vec<u64> = children.keys().copied().filter(|d| !related.contains(d)).collect();
    let [root] = roots.as_slice() else {
        return Err(fail(format!("{} root product definitions; exactly one is supported", roots.len())));
    };
    let bodies = data.raw_bodies()?.into_iter().map(|b| (b.id, b)).collect();
    let product = Product { data, rep_of, children, transform_of, bodies };
    let mut parts = Vec::new();
    product.walk(*root, Placement::IDENTITY, &mut Vec::new(), &mut parts, &mut BTreeSet::new())?;
    Ok(Assembly { root: *root, parts })
}
