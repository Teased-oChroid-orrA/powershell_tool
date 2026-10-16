//! Length-unit metadata only. Representation/body placement and actual geometry
//! conversion remain separate gates. Never infer a conversion factor from a label.
use crate::step::{Call, Entity, Error, ErrorKind, Inventory, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LengthContext {
    pub context: u64,
    pub unit: u64,
    /// Physical metres per unit in this assigned context.
    pub metres: f64,
}
fn error(entity: &Entity, detail: impl Into<String>) -> Error {
    Error {
        offset: entity.offset,
        kind: ErrorKind::Units,
        detail: detail.into(),
    }
}
fn component<'a>(e: &'a Entity, name: &str) -> Result<Option<&'a Call>, Error> {
    let mut found = e.components.iter().filter(|c| c.name == name);
    let first = found.next();
    if found.next().is_some() {
        return Err(error(
            e,
            format!("ambiguous {name} components in #{}", e.id),
        ));
    }
    Ok(first)
}
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(i) if i.unsigned_abs() <= 1u64 << 53 => Some(*i as f64),
        Value::Real(v) if v.is_finite() => Some(*v),
        _ => None,
    }
}
fn prefix(v: &Value) -> Option<f64> {
    match v {
        Value::Omitted => Some(1.),
        Value::Enum(s) => Some(match s.as_str() {
            "QUETTA" => 1e30,
            "RONNA" => 1e27,
            "YOTTA" => 1e24,
            "ZETTA" => 1e21,
            "EXA" => 1e18,
            "PETA" => 1e15,
            "TERA" => 1e12,
            "GIGA" => 1e9,
            "MEGA" => 1e6,
            "KILO" => 1e3,
            "HECTO" => 1e2,
            "DECA" => 1e1,
            "DECI" => 1e-1,
            "CENTI" => 1e-2,
            "MILLI" => 1e-3,
            "MICRO" => 1e-6,
            "NANO" => 1e-9,
            "PICO" => 1e-12,
            "FEMTO" => 1e-15,
            "ATTO" => 1e-18,
            "ZEPTO" => 1e-21,
            "YOCTO" => 1e-24,
            "RONTO" => 1e-27,
            "QUECTO" => 1e-30,
            _ => return None,
        }),
        _ => None,
    }
}
fn resolve(data: &Inventory, id: u64, active: &mut BTreeSet<u64>) -> Result<f64, Error> {
    // All references were checked by the inventory parser; still avoid indexing
    // unchecked data if this implementation is reused by a later adapter.
    let e = data.entities().get(&id).ok_or(Error {
        offset: 0,
        kind: ErrorKind::Units,
        detail: format!("missing unit #{id}"),
    })?;
    if active.len() >= 32 || !active.insert(id) {
        return Err(error(
            e,
            "cyclic or excessively deep length-unit conversion",
        ));
    }
    if e.components.iter().any(|c| {
        !matches!(
            c.name.as_str(),
            "LENGTH_UNIT" | "NAMED_UNIT" | "SI_UNIT" | "CONVERSION_BASED_UNIT"
        )
    }) {
        return Err(error(
            e,
            "unsupported length-unit subtype; offset/derived semantics must not be ignored",
        ));
    }
    if component(e, "LENGTH_UNIT")?.is_none() {
        return Err(error(
            e,
            "conversion basis is not declared as a LENGTH_UNIT",
        ));
    }
    let named =
        component(e, "NAMED_UNIT")?.ok_or_else(|| error(e, "missing NAMED_UNIT dimensions"))?;
    match named.args.as_slice() {
        [Value::Derived] => {}
        [Value::Ref(dim)] => {
            let d = &data.entities()[dim];
            let exponents = component(d, "DIMENSIONAL_EXPONENTS")?
                .ok_or_else(|| error(d, "unsupported unit dimensions"))?;
            if exponents.args.len() != 7
                || exponents
                    .args
                    .iter()
                    .enumerate()
                    .any(|(i, v)| number(v) != Some(if i == 0 { 1. } else { 0. }))
            {
                return Err(error(d, "dimensions do not describe length"));
            }
        }
        _ => return Err(error(e, "unsupported NAMED_UNIT dimension encoding")),
    }
    let si = component(e, "SI_UNIT")?;
    let converted = component(e, "CONVERSION_BASED_UNIT")?;
    let result = match (si, converted) {
        (Some(si), None) => match si.args.as_slice() {
            [p, Value::Enum(base)] if base == "METRE" => {
                prefix(p).ok_or_else(|| error(e, "unsupported SI prefix"))?
            }
            _ => return Err(error(e, "unsupported SI length-unit declaration")),
        },
        (None, Some(c)) => {
            let measure = match c.args.as_slice() {
                [Value::String(_), Value::Ref(id)] => &data.entities()[id],
                _ => return Err(error(e, "unsupported conversion-based unit declaration")),
            };
            let call = component(measure, "MEASURE_WITH_UNIT")?
                .or(component(measure, "LENGTH_MEASURE_WITH_UNIT")?)
                .ok_or_else(|| error(measure, "unsupported unit measure"))?;
            let (factor, basis) = match call.args.as_slice() {
                [Value::Typed(value), Value::Ref(basis)]
                    if value.name == "LENGTH_MEASURE" && value.args.len() == 1 =>
                {
                    (
                        number(&value.args[0]).ok_or_else(|| {
                            error(measure, "non-exact or unsupported conversion factor")
                        })?,
                        *basis,
                    )
                }
                _ => {
                    return Err(error(
                        measure,
                        "conversion requires an explicit LENGTH_MEASURE and unit reference",
                    ))
                }
            };
            if factor <= 0. {
                return Err(error(measure, "length conversion factor must be positive"));
            }
            factor * resolve(data, basis, active)?
        }
        _ => {
            return Err(error(
                e,
                "missing, conflicting or unsupported length-unit definition",
            ))
        }
    };
    active.remove(&id);
    if result.is_finite() && result > 0. {
        Ok(result)
    } else {
        Err(error(e, "length-unit conversion overflows or underflows"))
    }
}
pub fn contexts(data: &Inventory) -> Result<Vec<LengthContext>, Error> {
    let mut out = Vec::new();
    for e in data.entities().values() {
        if let Some(call) = component(e, "GLOBAL_UNIT_ASSIGNED_CONTEXT")? {
            let refs = match call.args.as_slice() {
                [Value::List(values)] => values,
                _ => return Err(error(e, "unsupported assigned-unit context")),
            };
            let mut length = None;
            for value in refs {
                let id = match value {
                    Value::Ref(id) => *id,
                    _ => return Err(error(e, "assigned units must be references")),
                };
                let unit = &data.entities()[&id];
                if component(unit, "LENGTH_UNIT")?.is_some() {
                    if length.is_some() {
                        return Err(error(e, "ambiguous assigned length units"));
                    }
                    length = Some(LengthContext {
                        context: e.id,
                        unit: id,
                        metres: resolve(data, id, &mut BTreeSet::new())?,
                    });
                }
            }
            out.push(
                length.ok_or_else(|| error(e, "no declared length unit in assigned context"))?,
            );
        }
    }
    if out.is_empty() {
        return Err(Error {
            offset: 0,
            kind: ErrorKind::Units,
            detail: "no assigned length-unit context; units are unknown".into(),
        });
    }
    Ok(out)
}
