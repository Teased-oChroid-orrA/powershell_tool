//! Persistence of the adapted-mesh store (`fea_core::adaptivity::MeshStore`, epic #62): one line per stored class, each with a
//! checksum, in the style of the benchmark store. Only what a start mesh needs is written: nodes, element blocks (kind,
//! connectivity, linear elastic material, density, name), node sets and named surfaces. A mesh with plasticity, heat data or an
//! anisotropic material is not persisted (counted in the report), so a reloaded mesh can never silently lose such data.
//!
//! Loading is a trust boundary: the schema is checked, the text and every record are bounded, a record whose checksum, key,
//! counts, connectivity, coordinates or material do not check out is dropped and counted, never repaired, and a loaded mesh is
//! only a *starting* mesh (the controller still solves and estimates on it).
use fea_core::adaptivity::{MeshStore, WorkloadKey};
use fea_core::element::ALL_KINDS;
use fea_core::{Elastic, Mesh, Physics};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: u32 = 1;
const MAX_STORE_BYTES: usize = 64 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
struct KeyRecord {
    geometry: String,
    material: String,
    boundary_conditions: String,
    contact: String,
    qois: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
struct BlockRecord {
    kind: String,
    conn: Vec<usize>,
    e: f64,
    nu: f64,
    alpha: f64,
    density: f64,
    name: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
struct MeshRecord {
    key: KeyRecord,
    /// "PlaneStress" | "PlaneStrain" | "Axisymmetric" | "Solid"
    physics: String,
    thickness: f64,
    nodes: Vec<[f64; 3]>,
    blocks: Vec<BlockRecord>,
    node_sets: BTreeMap<String, Vec<usize>>,
    surfaces: BTreeMap<String, Vec<Vec<usize>>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    check: String,
    record: MeshRecord,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EncodeReport {
    pub written: usize,
    /// Meshes with plasticity, heat data or an anisotropic material: not persisted.
    pub skipped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodeReport {
    pub loaded: usize,
    /// Records dropped for a bad checksum, key, structure or value.
    pub dropped: usize,
}

/// FNV-1a 64 over the record's canonical JSON: detects accidental corruption, it is not an authenticity claim.
fn checksum(record: &MeshRecord) -> Result<String, String> {
    let text = serde_json::to_string(record).map_err(|e| e.to_string())?;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(format!("{h:016x}"))
}

fn physics_of(m: &Mesh) -> (String, f64) {
    match m.physics {
        Physics::PlaneStress { thickness } => ("PlaneStress".into(), thickness),
        Physics::PlaneStrain { thickness } => ("PlaneStrain".into(), thickness),
        Physics::Axisymmetric => ("Axisymmetric".into(), 1.0),
        Physics::Solid => ("Solid".into(), 1.0),
    }
}

fn record_of(key: &WorkloadKey, m: &Mesh) -> Option<MeshRecord> {
    if m.blocks.iter().any(|b| b.plasticity.is_some() || b.thermal.is_some() || b.material.aniso.is_some()) {
        return None;
    }
    let (physics, thickness) = physics_of(m);
    Some(MeshRecord {
        key: KeyRecord { geometry: key.geometry.clone(), material: key.material.clone(), boundary_conditions: key.boundary_conditions.clone(), contact: key.contact.clone(), qois: key.qois.clone() },
        physics,
        thickness,
        nodes: m.nodes.clone(),
        blocks: m.blocks.iter().map(|b| BlockRecord { kind: format!("{:?}", b.kind), conn: b.conn.clone(), e: b.material.e, nu: b.material.nu, alpha: b.material.alpha, density: b.density, name: b.name.clone() }).collect(),
        node_sets: m.node_sets.clone(),
        surfaces: m.surfaces.clone(),
    })
}

fn mesh_of(r: &MeshRecord) -> Result<Mesh, String> {
    let physics = match r.physics.as_str() {
        "PlaneStress" if r.thickness.is_finite() && r.thickness > 0.0 => Physics::PlaneStress { thickness: r.thickness },
        "PlaneStrain" if r.thickness.is_finite() && r.thickness > 0.0 => Physics::PlaneStrain { thickness: r.thickness },
        "Axisymmetric" => Physics::Axisymmetric,
        "Solid" => Physics::Solid,
        other => return Err(format!("unknown physics {other:?}")),
    };
    if r.nodes.is_empty() || r.nodes.iter().flatten().any(|v| !v.is_finite()) {
        return Err("no nodes or a non-finite coordinate".into());
    }
    let mut mesh = Mesh::new(physics);
    for x in &r.nodes {
        mesh.add_node(*x);
    }
    if r.blocks.is_empty() {
        return Err("no element block".into());
    }
    for b in &r.blocks {
        let kind = ALL_KINDS.iter().copied().find(|k| format!("{k:?}") == b.kind).ok_or_else(|| format!("unknown element kind {:?}", b.kind))?;
        if kind.dim() != mesh.dim() || b.conn.is_empty() || b.conn.len() % kind.n_nodes() != 0 || b.conn.iter().any(|&n| n >= r.nodes.len()) {
            return Err(format!("block {:?}: dimension, connectivity length or node index is invalid", b.name));
        }
        if !(b.e.is_finite() && b.e > 0.0 && b.nu.is_finite() && b.nu > -1.0 && b.nu < 0.5 && b.alpha.is_finite() && b.density.is_finite() && b.density >= 0.0) {
            return Err(format!("block {:?}: invalid material", b.name));
        }
        let mut material = Elastic::new(b.e, b.nu);
        material.alpha = b.alpha;
        mesh.add_block(kind, b.conn.clone(), material, &b.name)?;
        mesh.blocks.last_mut().expect("just added").density = b.density;
    }
    for (name, set) in &r.node_sets {
        if set.iter().any(|&n| n >= r.nodes.len()) {
            return Err(format!("node set {name:?} has an index out of range"));
        }
    }
    for (name, faces) in &r.surfaces {
        if faces.iter().flatten().any(|&n| n >= r.nodes.len()) || faces.iter().any(|f| f.len() < 2) {
            return Err(format!("surface {name:?} has an invalid face"));
        }
    }
    mesh.node_sets = r.node_sets.clone();
    mesh.surfaces = r.surfaces.clone();
    Ok(mesh)
}

/// Serialize the store, one line per class after a schema line.
pub fn encode(store: &MeshStore) -> Result<(String, EncodeReport), String> {
    let mut out = format!("{{\"schema\":{SCHEMA}}}\n");
    let mut report = EncodeReport::default();
    for (key, mesh) in store.iter() {
        let Some(record) = record_of(key, mesh) else {
            report.skipped += 1;
            continue;
        };
        let line = Line { check: checksum(&record)?, record };
        let text = serde_json::to_string(&line).map_err(|e| e.to_string())?;
        if text.len() > MAX_RECORD_BYTES || out.len() + text.len() > MAX_STORE_BYTES || report.written >= MAX_ENTRIES {
            report.skipped += 1;
            continue;
        }
        out.push_str(&text);
        out.push('\n');
        report.written += 1;
    }
    Ok((out, report))
}

/// Rebuild a store; unknown schema or an oversized text is an error, bad records are dropped and counted.
pub fn decode(text: &str) -> Result<(MeshStore, DecodeReport), String> {
    if text.len() > MAX_STORE_BYTES {
        return Err("mesh store exceeds its byte budget".into());
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Header {
        schema: u32,
    }
    let mut lines = text.lines();
    let header: Header = serde_json::from_str(lines.next().ok_or("missing mesh store schema")?).map_err(|e| e.to_string())?;
    if header.schema != SCHEMA {
        return Err("unsupported mesh store schema; explicit migration required".into());
    }
    let (mut store, mut report) = (MeshStore::new(), DecodeReport::default());
    for line in lines {
        if report.loaded >= MAX_ENTRIES {
            report.dropped += 1;
            continue;
        }
        let parsed = (line.len() <= MAX_RECORD_BYTES).then(|| serde_json::from_str::<Line>(line).ok()).flatten();
        let ok = parsed.and_then(|l| (checksum(&l.record).ok().as_deref() == Some(l.check.as_str())).then_some(l.record)).and_then(|r| {
            let mesh = mesh_of(&r).ok()?;
            let key = WorkloadKey { geometry: r.key.geometry, material: r.key.material, boundary_conditions: r.key.boundary_conditions, contact: r.key.contact, qois: r.key.qois };
            Some((key, mesh))
        });
        match ok {
            Some((key, mesh)) => {
                store.insert(key, mesh);
                report.loaded += 1;
            }
            None => report.dropped += 1,
        }
    }
    Ok((store, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fea_core::generate::grid;
    use fea_core::ElementKind;

    fn key(n: &str) -> WorkloadKey {
        WorkloadKey { geometry: n.into(), material: "E=1e7,nu=0.3".into(), boundary_conditions: "fixed left".into(), contact: "none".into(), qois: vec!["sigma".into()] }
    }

    fn mesh(kind: ElementKind, div: [usize; 3]) -> Mesh {
        let physics = if kind.dim() == 2 { Physics::PlaneStress { thickness: 2.5 } } else { Physics::Solid };
        let mut m = grid(physics, kind, Elastic::new(1.0e7, 0.3), div, &|p| p).unwrap();
        m.blocks[0].density = 7.8e-9;
        m
    }

    fn same(a: &Mesh, b: &Mesh) -> bool {
        a.physics == b.physics
            && a.nodes == b.nodes
            && a.blocks.len() == b.blocks.len()
            && a.blocks.iter().zip(&b.blocks).all(|(x, y)| x.kind == y.kind && x.conn == y.conn && x.material.e == y.material.e && x.material.nu == y.material.nu && x.density == y.density && x.name == y.name)
            && a.node_sets == b.node_sets
            && a.surfaces == b.surfaces
    }

    #[test]
    fn a_store_round_trips_bit_exactly_and_the_classes_stay_apart() {
        let mut store = MeshStore::new();
        let (a, b) = (mesh(ElementKind::Quad8, [3, 2, 1]), mesh(ElementKind::Hex8, [2, 2, 2]));
        store.insert(key("plate"), a.clone());
        store.insert(key("block"), b.clone());
        let (text, enc) = encode(&store).unwrap();
        assert_eq!(enc, EncodeReport { written: 2, skipped: 0 });
        let (back, dec) = decode(&text).unwrap();
        assert_eq!(dec, DecodeReport { loaded: 2, dropped: 0 });
        assert!(same(back.recall(&key("plate")).unwrap(), &a) && same(back.recall(&key("block")).unwrap(), &b));
        assert!(back.recall(&key("other")).is_none());
        // The reloaded mesh builds a model.
        assert!(fea_core::Model::new(back.recall(&key("block")).unwrap().clone()).is_ok());
    }

    #[test]
    fn corrupt_tampered_and_unsupported_input_is_dropped_or_refused_never_repaired() {
        let mut store = MeshStore::new();
        store.insert(key("plate"), mesh(ElementKind::Quad4, [3, 3, 1]));
        store.insert(key("other"), mesh(ElementKind::Tri3, [2, 2, 1]));
        let (text, _) = encode(&store).unwrap();
        // A changed coordinate breaks the checksum.
        let tampered = text.replacen("\"nodes\":[[0.0,0.0,0.0]", "\"nodes\":[[0.5,0.0,0.0]", 1);
        assert_ne!(tampered, text);
        let (s, r) = decode(&tampered).unwrap();
        assert_eq!((s.len(), r.loaded, r.dropped), (1, 1, 1), "{r:?}");
        // A connectivity index out of range, with a recomputed (consistent) checksum, is refused by structure.
        let mut line: Line = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
        line.record.blocks[0].conn[0] = 9999;
        line.check = checksum(&line.record).unwrap();
        let bad = format!("{{\"schema\":1}}\n{}\n", serde_json::to_string(&line).unwrap());
        assert_eq!(decode(&bad).unwrap().1, DecodeReport { loaded: 0, dropped: 1 });
        // A non-finite or invalid material, an unknown kind, a truncated line and garbage.
        for edit in [|l: &mut Line| l.record.blocks[0].e = -1.0, |l: &mut Line| l.record.blocks[0].kind = "Tet99".into(), |l: &mut Line| l.record.nodes[0][0] = f64::NAN] {
            let mut l: Line = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
            edit(&mut l);
            l.check = checksum(&l.record).unwrap();
            let t = format!("{{\"schema\":1}}\n{}\n", serde_json::to_string(&l).unwrap_or_default());
            assert_eq!(decode(&t).unwrap().1.loaded, 0);
        }
        let truncated = &text[..text.len() / 2];
        assert_eq!(decode(truncated).unwrap().1.loaded, 0);
        assert!(decode("not json").is_err());
        assert!(decode("{\"schema\":2}\n").is_err());
        assert!(decode("").is_err());
    }

    #[test]
    fn meshes_with_data_the_file_cannot_carry_are_not_persisted() {
        let mut store = MeshStore::new();
        let mut m = mesh(ElementKind::Quad4, [2, 2, 1]);
        m.blocks[0].plasticity = Some(fea_core::J2::elastic_finite());
        store.insert(key("plastic"), m);
        store.insert(key("elastic"), mesh(ElementKind::Quad4, [2, 2, 1]));
        let (text, enc) = encode(&store).unwrap();
        assert_eq!(enc, EncodeReport { written: 1, skipped: 1 });
        let (back, _) = decode(&text).unwrap();
        assert!(back.recall(&key("plastic")).is_none() && back.recall(&key("elastic")).is_some());
    }
}
