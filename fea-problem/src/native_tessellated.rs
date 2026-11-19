//! A STEP `TESSELLATED_SOLID` (AP242 triangulated geometry, no B-rep) read into the face-tagged surface cache.
//!
//! The faces are `COMPLEX_TRIANGULATED_FACE`s (strips and fans) and `TRIANGULATED_FACE`s over a `COORDINATES_LIST`; `pn_index`
//! maps a face's point positions to coordinates, strips and fans index `pn_index`. Faces that share a position are welded into one
//! vertex so the shell closes across faces. Face `k` of the cache is the `k`-th face of the solid (1-based in the editor). A
//! tessellated solid has no analytic surfaces or edges: it meshes and solves, but a B-rep edit (hole, fillet, ...) refuses.
use crate::solid::{finish_import, SolidCache, StepImport};
use solid_kernel::step::{inventory, Entity, Inventory, Limits, Value};
use solid_kernel::SurfaceMesh;
use std::collections::HashMap;

/// Whether `step` holds a tessellated solid and no B-rep solid for the B-rep reader to take.
pub fn is_tessellated_solid(step: &str) -> bool {
    inventory(step, Limits::default()).is_ok_and(|d| d.counts().get("TESSELLATED_SOLID").is_some_and(|n| *n > 0) && d.counts().get("MANIFOLD_SOLID_BREP").is_none_or(|n| *n == 0))
}

fn entity(d: &Inventory, id: u64) -> Result<&Entity, String> {
    d.entities().get(&id).ok_or_else(|| format!("tessellated STEP: entity #{id} is missing"))
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Real(x) => Some(*x),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

fn index(v: &Value) -> Option<usize> {
    match v {
        Value::Integer(i) if *i >= 1 => Some(*i as usize),
        _ => None,
    }
}

fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(l) => l,
        _ => &[],
    }
}

fn indices(v: &Value) -> Result<Vec<usize>, String> {
    list(v).iter().map(|x| index(x).ok_or_else(|| "tessellated STEP: an index is not a positive integer".to_string())).collect()
}

/// The faces of `solid`'s items, expanding a `TESSELLATED_SHELL` into its own.
fn face_ids(d: &Inventory, items: &[Value], out: &mut Vec<u64>) -> Result<(), String> {
    for it in items {
        let Value::Ref(id) = it else { return Err("tessellated STEP: a solid item is not an entity reference".into()) };
        let e = entity(d, *id)?;
        match e.components.first().map(|c| c.name.as_str()) {
            Some("TESSELLATED_SHELL") => face_ids(d, e.components[0].args.get(1).map_or(&[][..], list), out)?,
            Some("COMPLEX_TRIANGULATED_FACE" | "TRIANGULATED_FACE") => out.push(*id),
            other => return Err(format!("tessellated STEP: unsupported solid item {other:?}")),
        }
    }
    Ok(())
}

/// Triangles `(a, b, c)` of one strip (alternating orientation) or fan, as positions.
fn strip(p: &[usize], out: &mut Vec<[usize; 3]>) {
    for k in 0..p.len().saturating_sub(2) {
        out.push(if k % 2 == 0 { [p[k], p[k + 1], p[k + 2]] } else { [p[k + 1], p[k], p[k + 2]] });
    }
}

fn fan(p: &[usize], out: &mut Vec<[usize; 3]>) {
    for k in 1..p.len().saturating_sub(1) {
        out.push([p[0], p[k], p[k + 1]]);
    }
}

/// The face-tagged, welded, outward-oriented surface of the tessellated solid in `step`, in millimetres, and its bounds.
pub fn import_tessellated(step: &str) -> Result<StepImport, String> {
    let d = inventory(step, Limits::default()).map_err(|e| e.to_string())?;
    let solids: Vec<&Entity> = d.entities().values().filter(|e| e.components.iter().any(|c| c.name == "TESSELLATED_SOLID")).collect();
    let [solid] = solids[..] else { return Err(format!("tessellated STEP: exactly one tessellated solid is required, found {}", solids.len())) };
    let mut faces = Vec::new();
    let call = solid.components.iter().find(|c| c.name == "TESSELLATED_SOLID").unwrap();
    face_ids(&d, call.args.get(1).map_or(&[][..], list), &mut faces)?;
    if faces.is_empty() {
        return Err("tessellated STEP: the solid has no faces".into());
    }
    // metres per unit of the representation's length context; a file with several contexts is ambiguous
    let scale_mm = match d.length_contexts().map_err(|e| e.to_string())?.as_slice() {
        [] => 1.0,
        [c] => c.metres * 1000.0,
        _ => return Err("tessellated STEP: several length contexts".into()),
    };
    let mut lists: HashMap<u64, Vec<[f64; 3]>> = HashMap::new();
    let mut welded: HashMap<[i64; 3], usize> = HashMap::new();
    let mut mesh = SurfaceMesh::default();
    const GRID: f64 = 1e-5; // mm: far below any tessellation, above print round-off
    for (face, id) in faces.iter().enumerate() {
        let c = entity(&d, *id)?.components.iter().find(|c| matches!(c.name.as_str(), "COMPLEX_TRIANGULATED_FACE" | "TRIANGULATED_FACE")).unwrap();
        let Some(Value::Ref(coords)) = c.args.get(1) else { return Err("tessellated STEP: a face has no coordinates list".into()) };
        if !lists.contains_key(coords) {
            let e = entity(&d, *coords)?;
            let cl = e.components.iter().find(|c| c.name == "COORDINATES_LIST").ok_or("tessellated STEP: coordinates are not a COORDINATES_LIST")?;
            let pts = list(cl.args.get(2).ok_or("tessellated STEP: COORDINATES_LIST has no points")?)
                .iter()
                .map(|p| match list(p).iter().map(num).collect::<Option<Vec<_>>>().as_deref() {
                    Some([x, y, z]) => Ok([x * scale_mm, y * scale_mm, z * scale_mm]),
                    _ => Err("tessellated STEP: a coordinate is not three numbers".to_string()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            lists.insert(*coords, pts);
        }
        let pts = &lists[coords];
        let pn = indices(c.args.get(5).ok_or("tessellated STEP: a face has no pn_index")?)?;
        let position = |k: usize| -> Result<usize, String> {
            let at = if pn.is_empty() { k } else { *pn.get(k - 1).ok_or("tessellated STEP: a strip, fan or triangle index is past pn_index")? };
            if at <= pts.len() { Ok(at - 1) } else { Err("tessellated STEP: a pn_index entry is past the coordinates list".to_string()) }
        };
        let mut tris: Vec<[usize; 3]> = Vec::new();
        if c.name == "COMPLEX_TRIANGULATED_FACE" {
            for s in c.args.get(6).map_or(&[][..], list) { strip(&indices(s)?, &mut tris); }
            for f in c.args.get(7).map_or(&[][..], list) { fan(&indices(f)?, &mut tris); }
        } else {
            for t in c.args.get(6).map_or(&[][..], list) {
                let t = indices(t)?;
                let [a, b, cc] = t[..] else { return Err("tessellated STEP: a triangle has not three indices".into()) };
                tris.push([a, b, cc]);
            }
        }
        for t in tris {
            let mut v = [0usize; 3];
            for (k, p) in t.iter().enumerate() {
                let q = pts[position(*p)?];
                let key = q.map(|x| (x / GRID).round() as i64);
                v[k] = *welded.entry(key).or_insert_with(|| {
                    mesh.vertices.push(q);
                    mesh.vertices.len() - 1
                });
            }
            if v[0] != v[1] && v[1] != v[2] && v[0] != v[2] {
                mesh.triangles.push(v);
                mesh.face.push(face);
            }
        }
    }
    if mesh.triangles.len() < 4 {
        return Err("tessellated STEP: fewer than four triangles".into());
    }
    // outward orientation: a negative signed volume means the whole shell is wound inward
    let volume: f64 = mesh.triangles.iter().map(|t| {
        let [a, b, c] = t.map(|i| mesh.vertices[i]);
        (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
    }).sum();
    if volume < 0.0 {
        for t in &mut mesh.triangles { t.swap(1, 2); }
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for v in &mesh.vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    if !(diag.is_finite() && diag > 0.0) {
        return Err("the tessellated solid has no extent".into());
    }
    Ok(finish_import(SolidCache::new(step, mesh, diag * 1e-3), lo, hi, diag))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10 mm cube as a tessellated solid: five `TRIANGULATED_FACE`s and one `COMPLEX_TRIANGULATED_FACE` (a fan). `inward`
    /// reverses every winding.
    fn cube(inward: bool) -> String {
        let pts = "(0.,0.,0.),(10.,0.,0.),(10.,10.,0.),(0.,10.,0.),(0.,0.,10.),(10.,0.,10.),(10.,10.,10.),(0.,10.,10.)";
        let faces: [&[[usize; 3]]; 5] = [&[[1, 3, 2], [1, 4, 3]], &[[5, 6, 7], [5, 7, 8]], &[[1, 2, 6], [1, 6, 5]], &[[4, 8, 7], [4, 7, 3]], &[[1, 5, 8], [1, 8, 4]]];
        let mut s = String::from("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('AP242'));\nENDSEC;\nDATA;\n");
        s += "#61=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\n#96=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#61))REPRESENTATION_CONTEXT(' ',' '));\n";
        s += &format!("#1=COORDINATES_LIST('',8,({pts}));\n");
        let pn = "(1,2,3,4,5,6,7,8)";
        for (k, f) in faces.iter().enumerate() {
            let t: Vec<String> = f.iter().map(|t| if inward { format!("({},{},{})", t[0], t[2], t[1]) } else { format!("({},{},{})", t[0], t[1], t[2]) }).collect();
            s += &format!("#{}=TRIANGULATED_FACE('',#1,8,(),$,{pn},({}));\n", 10 + k, t.join(","));
        }
        let fan = if inward { "(2,6,7,3)" } else { "(2,3,7,6)" };
        s += &format!("#20=COMPLEX_TRIANGULATED_FACE('',#1,8,(),$,{pn},(),({fan}));\n");
        s += "#30=TESSELLATED_SOLID('cube',(#10,#11,#12,#13,#14,#20),$);\nENDSEC;\nEND-ISO-10303-21;\n";
        s
    }

    fn volume(i: &StepImport) -> f64 {
        i.cache.triangles.iter().map(|t| {
            let [a, b, c] = t.map(|k| i.cache.vertices[k]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
        }).sum()
    }

    #[test]
    fn a_tessellated_cube_reads_welded_tagged_and_outward() {
        for inward in [false, true] {
            let text = cube(inward);
            assert!(is_tessellated_solid(&text));
            let i = import_tessellated(&text).unwrap();
            assert_eq!((i.cache.vertices.len(), i.cache.triangles.len()), (8, 12), "inward {inward}: welded vertices and triangles");
            assert!((volume(&i) - 1000.0).abs() < 1e-9, "inward {inward}: volume {}", volume(&i));
            let mut faces = i.cache.face.clone();
            faces.dedup();
            assert_eq!(faces.len(), 6, "six faces, two triangles each");
            // the same entry point as the application's
            assert!(crate::native_solid::import_step_native(&text).is_ok());
        }
    }

    #[test]
    fn a_strip_alternates_orientation_and_a_bad_index_refuses() {
        let (mut out, mut p) = (Vec::new(), [1usize, 2, 3, 4]);
        strip(&p, &mut out);
        assert_eq!(out, vec![[1, 2, 3], [3, 2, 4]]);
        out.clear();
        fan(&p, &mut out);
        assert_eq!(out, vec![[1, 2, 3], [1, 3, 4]]);
        p[3] = 9;
        let bad = cube(false).replace("(2,3,7,6)", "(2,3,7,9)");
        assert!(import_tessellated(&bad).unwrap_err().contains("past"));
    }
}
