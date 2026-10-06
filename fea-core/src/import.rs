//! Mesh import from text: Gmsh `.msh` (ASCII, format 4.1 and 2.2) and Abaqus `.inp`. Pure functions
//! from text to [`Mesh`]; callers do the file I/O.
//!
//! Supported element types: Tri3/6, Quad4/8/9, Tet4/10, Hex8/20/27 (Gmsh types 2, 9, 3, 16, 10, 4,
//! 11, 5, 17, 12; Abaqus `CPS/CPE/CAX` 3/4/6/8-node and `C3D4/10/8/20`). Quadratic node orders are
//! permuted to the library's VTK order and **checked geometrically**: every midside / face-centre
//! node must lie near the position its corner nodes imply, so a node-ordering mismatch is an error
//! instead of a silently distorted mesh.
//!
//! Gmsh: physical groups become blocks (volume entities) and named node sets / surfaces (lower
//! dimension); surfaces are the boundary faces of the volume mesh matching the surface elements, in
//! load orientation. Abaqus: element sets become blocks with the material of their `*SOLID
//! SECTION`; node sets are imported; surfaces can be derived with `Mesh::select_faces_by_nodes`.

use crate::element::ElementKind;
use crate::mesh::{Elastic, Mesh, Physics};
use std::collections::{BTreeMap, HashMap};

/// What to build a mesh with when the file does not say.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Analysis type for 2D meshes (ignored for 3D). Abaqus `CPS/CPE/CAX` types override it.
    pub physics: Physics,
    /// Material of elements without a specific one.
    pub material: Elastic,
    /// Gmsh physical-group name to material.
    pub materials: HashMap<String, Elastic>,
}

impl ImportOptions {
    pub fn new(physics: Physics, material: Elastic) -> Self {
        Self { physics, material, materials: HashMap::new() }
    }
}

/// `VTK[k] = file[perm[k]]` for the types whose file order differs from VTK.
fn gmsh_permutation(kind: ElementKind) -> Option<&'static [usize]> {
    match kind {
        ElementKind::Tet10 => Some(&[0, 1, 2, 3, 4, 5, 6, 7, 9, 8]),
        ElementKind::Hex20 => Some(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 11, 13, 9, 16, 18, 19, 17, 10, 12, 14, 15]),
        ElementKind::Hex27 => Some(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 11, 13, 9, 16, 18, 19, 17, 10, 12, 14, 15, 22, 23, 21, 24, 20, 25, 26]),
        _ => None,
    }
}

fn gmsh_kind(t: u32) -> Option<ElementKind> {
    Some(match t {
        2 => ElementKind::Tri3,
        9 => ElementKind::Tri6,
        3 => ElementKind::Quad4,
        16 => ElementKind::Quad8,
        10 => ElementKind::Quad9,
        4 => ElementKind::Tet4,
        11 => ElementKind::Tet10,
        5 => ElementKind::Hex8,
        17 => ElementKind::Hex20,
        12 => ElementKind::Hex27,
        _ => return None,
    })
}

fn num(tok: &str) -> Result<f64, String> {
    tok.replace(['d', 'D'], "e").parse::<f64>().map_err(|_| format!("'{tok}' is not a number"))
}

fn idx(tok: &str) -> Result<i64, String> {
    tok.parse::<i64>().map_err(|_| format!("'{tok}' is not an integer"))
}

/// Verify that every non-corner node of every element sits near the position implied by its corners.
fn check_node_order(mesh: &Mesh, source: &str) -> Result<(), String> {
    for blk in &mesh.blocks {
        let kind = blk.kind;
        if kind.n_nodes() == kind.n_corners() {
            continue;
        }
        let lin = match kind.dim() {
            2 if kind.is_simplex() => ElementKind::Tri3,
            2 => ElementKind::Quad4,
            _ if kind.is_simplex() => ElementKind::Tet4,
            _ => ElementKind::Hex8,
        };
        let nc = kind.n_corners();
        for (e, conn) in blk.conn.chunks_exact(kind.n_nodes()).enumerate() {
            let corners: Vec<[f64; 3]> = conn[..nc].iter().map(|&n| mesh.nodes[n]).collect();
            let mut size = 0.0f64;
            for a in 0..nc {
                for b in a + 1..nc {
                    size = size.max((0..3).map(|i| (corners[a][i] - corners[b][i]).powi(2)).sum::<f64>().sqrt());
                }
            }
            for (k, xi) in kind.node_coords().iter().enumerate().skip(nc) {
                let (n, _) = lin.shape(*xi);
                let want: [f64; 3] = std::array::from_fn(|i| (0..nc).map(|a| n[a] * corners[a][i]).sum());
                let got = mesh.nodes[conn[k]];
                let dist = (0..3).map(|i| (got[i] - want[i]).powi(2)).sum::<f64>().sqrt();
                if dist > 0.3 * size {
                    return Err(format!("{source}: {kind:?} element {e} node {k} is {dist:.3e} from where its corners put it (element size {size:.3e}): node ordering mismatch"));
                }
            }
        }
    }
    Ok(())
}

// ====================================================================== Gmsh

struct GmshElement {
    dim: usize,
    entity: i64,
    kind: Option<ElementKind>,
    /// Raw type number for diagnostics.
    ty: u32,
    nodes: Vec<usize>,
}

/// Read a Gmsh `.msh` (ASCII 4.1 or 2.2).
pub fn read_gmsh(text: &str, opt: &ImportOptions) -> Result<Mesh, String> {
    let mut sections: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut cur: Option<(&str, Vec<&str>)> = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(name) = l.strip_prefix('$') {
            if let Some(end) = name.strip_prefix("End") {
                if let Some((n, body)) = cur.take() {
                    if n != end {
                        return Err(format!("section ${n} closed by $End{end}"));
                    }
                    sections.insert(n, body);
                }
            } else {
                cur = Some((name, Vec::new()));
            }
        } else if let Some((_, body)) = cur.as_mut() {
            if !l.is_empty() {
                body.push(l);
            }
        }
    }
    let fmt = sections.get("MeshFormat").and_then(|b| b.first()).ok_or("no $MeshFormat section")?;
    let mut ft = fmt.split_whitespace();
    let version = ft.next().ok_or("empty $MeshFormat")?;
    if ft.next() != Some("0") {
        return Err("binary Gmsh files are not supported (save as ASCII)".into());
    }
    let v4 = version.starts_with("4.1");
    if !v4 && !version.starts_with("2.") {
        return Err(format!("Gmsh format {version} is not supported (use 4.1 or 2.2 ASCII)"));
    }
    // Physical names: (dim, tag) -> name.
    let mut pnames: HashMap<(usize, i64), String> = HashMap::new();
    if let Some(b) = sections.get("PhysicalNames") {
        for l in b.iter().skip(1) {
            let mut it = l.splitn(3, char::is_whitespace);
            let (d, t, rest) = (idx(it.next().unwrap_or(""))?, idx(it.next().unwrap_or(""))?, it.next().unwrap_or("").trim());
            pnames.insert((d as usize, t), rest.trim_matches('"').to_string());
        }
    }
    // Entity (dim, tag) -> physical tags (4.1 only).
    let mut entity_phys: HashMap<(usize, i64), Vec<i64>> = HashMap::new();
    if v4 {
        if let Some(b) = sections.get("Entities") {
            let mut it = b[0].split_whitespace();
            let counts: Vec<usize> = (0..4).map(|_| it.next().ok_or("bad $Entities header").and_then(|t| t.parse::<usize>().map_err(|_| "bad $Entities header"))).collect::<Result<_, _>>()?;
            let mut li = 1;
            for (dim, &n) in counts.iter().enumerate() {
                for _ in 0..n {
                    let t: Vec<&str> = b.get(li).ok_or("truncated $Entities")?.split_whitespace().collect();
                    li += 1;
                    let tag = idx(t[0])?;
                    let skip = if dim == 0 { 4 } else { 7 }; // tag + (x y z) or (min xyz max xyz)
                    let nphys = t.get(skip).map(|s| s.parse::<usize>().unwrap_or(0)).unwrap_or(0);
                    let phys = (0..nphys).map(|k| idx(t[skip + 1 + k])).collect::<Result<Vec<_>, _>>()?;
                    entity_phys.insert((dim, tag), phys);
                }
            }
        }
    }
    // Nodes.
    let nb = sections.get("Nodes").ok_or("no $Nodes section")?;
    let mut tag_of: HashMap<i64, usize> = HashMap::new();
    let mut coords: Vec<[f64; 3]> = Vec::new();
    if v4 {
        let h: Vec<&str> = nb[0].split_whitespace().collect();
        let blocks = idx(h[0])? as usize;
        let mut li = 1;
        for _ in 0..blocks {
            let bh: Vec<&str> = nb.get(li).ok_or("truncated $Nodes")?.split_whitespace().collect();
            let count = idx(bh.get(3).ok_or("bad node block header")?)? as usize;
            if bh.get(2) == Some(&"1") {
                return Err("parametric node blocks are not supported".into());
            }
            li += 1;
            let tags: Vec<i64> = nb[li..li + count].iter().map(|l| idx(l.split_whitespace().next().unwrap_or(""))).collect::<Result<_, _>>()?;
            li += count;
            for (k, l) in nb[li..li + count].iter().enumerate() {
                let c: Vec<&str> = l.split_whitespace().collect();
                if c.len() < 3 {
                    return Err("a node needs three coordinates".into());
                }
                tag_of.insert(tags[k], coords.len());
                coords.push([num(c[0])?, num(c[1])?, num(c[2])?]);
            }
            li += count;
        }
    } else {
        for l in nb.iter().skip(1) {
            let c: Vec<&str> = l.split_whitespace().collect();
            tag_of.insert(idx(c[0])?, coords.len());
            coords.push([num(c[1])?, num(c[2])?, num(c[3])?]);
        }
    }
    // Elements.
    let eb = sections.get("Elements").ok_or("no $Elements section")?;
    let mut elems: Vec<GmshElement> = Vec::new();
    let node_ids = |toks: &[&str]| -> Result<Vec<usize>, String> { toks.iter().map(|t| idx(t).and_then(|v| tag_of.get(&v).copied().ok_or(format!("element refers to missing node {v}")))).collect() };
    if v4 {
        let h: Vec<&str> = eb[0].split_whitespace().collect();
        let blocks = idx(h[0])? as usize;
        let mut li = 1;
        for _ in 0..blocks {
            let bh: Vec<&str> = eb.get(li).ok_or("truncated $Elements")?.split_whitespace().collect();
            let (dim, ent, ty, count) = (idx(bh[0])? as usize, idx(bh[1])?, idx(bh[2])? as u32, idx(bh[3])? as usize);
            li += 1;
            for l in &eb[li..li + count] {
                let t: Vec<&str> = l.split_whitespace().collect();
                elems.push(GmshElement { dim, entity: ent, kind: gmsh_kind(ty), ty, nodes: node_ids(&t[1..])? });
            }
            li += count;
        }
    } else {
        for l in eb.iter().skip(1) {
            let t: Vec<&str> = l.split_whitespace().collect();
            let ty = idx(t[1])? as u32;
            let ntags = idx(t[2])? as usize;
            let phys = if ntags > 0 { idx(t[3])? } else { 0 };
            let dim = match ty {
                15 => 0,
                1 | 8 => 1,
                2 | 3 | 9 | 10 | 16 => 2,
                _ => 3,
            };
            // Version 2 carries the physical tag per element; fold it into the entity key.
            elems.push(GmshElement { dim, entity: phys, kind: gmsh_kind(ty), ty, nodes: node_ids(&t[3 + ntags..])? });
        }
    }
    let name_of = |dim: usize, entity: i64| -> String {
        let tags = if v4 { entity_phys.get(&(dim, entity)).cloned().unwrap_or_default() } else { vec![entity] };
        tags.first().and_then(|t| pnames.get(&(dim, *t)).cloned()).unwrap_or_else(|| if v4 && !tags.is_empty() { format!("physical_{dim}_{}", tags[0]) } else { "domain".to_string() })
    };
    let top = elems.iter().filter(|e| e.dim >= 2).map(|e| e.dim).max().ok_or("the file contains no surface or volume elements")?;
    if let Some(e) = elems.iter().find(|e| e.dim == top && e.kind.is_none()) {
        return Err(format!("unsupported element type {} (prisms and pyramids have no element in this library)", e.ty));
    }
    let physics = if top == 3 { Physics::Solid } else { opt.physics };
    if top == 2 && physics.dim() != 2 {
        return Err("a surface mesh needs a 2D analysis type".into());
    }
    // Only nodes of retained elements enter the mesh (renumbered densely, file order).
    let mut mesh = Mesh::new(physics);
    let mut new_id: HashMap<usize, usize> = HashMap::new();
    let mut take = |mesh: &mut Mesh, n: usize| -> usize {
        *new_id.entry(n).or_insert_with(|| {
            let c = coords[n];
            mesh.add_node(if physics.dim() == 2 { [c[0], c[1], 0.0] } else { c })
        })
    };
    // Blocks: (physical name, kind) in first-seen order.
    let mut block_order: Vec<(String, ElementKind)> = Vec::new();
    let mut block_conn: HashMap<(String, ElementKind), Vec<usize>> = HashMap::new();
    for e in elems.iter().filter(|e| e.dim == top) {
        let kind = e.kind.unwrap();
        let name = name_of(e.dim, e.entity);
        let perm = gmsh_permutation(kind);
        let conn = block_conn.entry((name.clone(), kind)).or_insert_with(|| {
            block_order.push((name.clone(), kind));
            Vec::new()
        });
        for k in 0..kind.n_nodes() {
            conn.push(take(&mut mesh, e.nodes[perm.map_or(k, |p| p[k])]));
        }
    }
    for (name, kind) in block_order {
        let conn = block_conn.remove(&(name.clone(), kind)).unwrap();
        let mat = opt.materials.get(&name).copied().unwrap_or(opt.material);
        mesh.add_block(kind, conn, mat, &name)?;
    }
    check_node_order(&mesh, "Gmsh import")?;
    // Lower-dimension physical groups: node sets and surfaces.
    let faces = mesh.boundary_faces();
    let mut by_corners: HashMap<Vec<usize>, usize> = HashMap::new();
    let ncorner = |n: usize| -> usize {
        if top == 2 {
            2
        } else if matches!(n, 3 | 6) {
            3
        } else {
            4
        }
    };
    for (i, f) in faces.iter().enumerate() {
        let mut k: Vec<usize> = f.nodes[..ncorner(f.nodes.len())].to_vec();
        k.sort_unstable();
        by_corners.insert(k, i);
    }
    let mut sets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut surfs: BTreeMap<String, Vec<Vec<usize>>> = BTreeMap::new();
    for e in elems.iter().filter(|e| e.dim + 1 == top && e.dim >= 1) {
        // Corner count of the boundary element: a face has its kind's corners, a line (types 1, 8) two.
        let nc = match e.kind {
            Some(k) => k.n_corners(),
            None if matches!(e.ty, 1 | 8) => 2,
            None => continue,
        };
        let ids: Vec<usize> = e.nodes.iter().filter_map(|n| new_id.get(n).copied()).collect();
        if ids.len() != e.nodes.len() {
            continue; // a boundary element not attached to the retained mesh
        }
        let name = name_of(e.dim, e.entity);
        sets.entry(name.clone()).or_default().extend(&ids);
        let mut k: Vec<usize> = ids[..nc].to_vec();
        k.sort_unstable();
        if let Some(&fi) = by_corners.get(&k) {
            surfs.entry(name).or_default().push(faces[fi].nodes.clone());
        }
    }
    for (n, mut v) in sets {
        v.sort_unstable();
        v.dedup();
        mesh.node_sets.insert(n, v);
    }
    mesh.surfaces.extend(surfs);
    Ok(mesh)
}

// ====================================================================== Abaqus

fn abaqus_kind(ty: &str) -> Option<(ElementKind, Option<Physics>)> {
    let t = ty.to_ascii_uppercase();
    let t = t.trim_end_matches(['R', 'H', 'I']); // reduced / hybrid / incompatible variants
    let plane = |p| Some(p);
    Some(match t {
        "CPS3" => (ElementKind::Tri3, plane(Physics::PlaneStress { thickness: 1.0 })),
        "CPS4" => (ElementKind::Quad4, plane(Physics::PlaneStress { thickness: 1.0 })),
        "CPS6" => (ElementKind::Tri6, plane(Physics::PlaneStress { thickness: 1.0 })),
        "CPS8" => (ElementKind::Quad8, plane(Physics::PlaneStress { thickness: 1.0 })),
        "CPE3" => (ElementKind::Tri3, plane(Physics::PlaneStrain { thickness: 1.0 })),
        "CPE4" => (ElementKind::Quad4, plane(Physics::PlaneStrain { thickness: 1.0 })),
        "CPE6" => (ElementKind::Tri6, plane(Physics::PlaneStrain { thickness: 1.0 })),
        "CPE8" => (ElementKind::Quad8, plane(Physics::PlaneStrain { thickness: 1.0 })),
        "CAX3" => (ElementKind::Tri3, plane(Physics::Axisymmetric)),
        "CAX4" => (ElementKind::Quad4, plane(Physics::Axisymmetric)),
        "CAX6" => (ElementKind::Tri6, plane(Physics::Axisymmetric)),
        "CAX8" => (ElementKind::Quad8, plane(Physics::Axisymmetric)),
        "C3D4" => (ElementKind::Tet4, None),
        "C3D10" => (ElementKind::Tet10, None),
        "C3D8" => (ElementKind::Hex8, None),
        "C3D20" => (ElementKind::Hex20, None),
        _ => return None,
    })
}

/// Split a keyword line into its upper-case keyword and `NAME=value` parameters.
fn keyword(line: &str) -> (String, HashMap<String, String>, Vec<String>) {
    let mut parts = line.trim_start_matches('*').split(',').map(str::trim);
    let kw = parts.next().unwrap_or("").to_ascii_uppercase();
    let mut params = HashMap::new();
    let mut flags = Vec::new();
    for p in parts {
        match p.split_once('=') {
            Some((k, v)) => {
                params.insert(k.trim().to_ascii_uppercase(), v.trim().to_string());
            }
            None if !p.is_empty() => flags.push(p.to_ascii_uppercase()),
            None => {}
        }
    }
    (kw, params, flags)
}

/// Read an Abaqus `.inp`.
pub fn read_abaqus(text: &str, opt: &ImportOptions) -> Result<Mesh, String> {
    struct Elem {
        id: i64,
        kind: ElementKind,
        physics: Option<Physics>,
        nodes: Vec<i64>,
    }
    let mut nodes: Vec<(i64, [f64; 3])> = Vec::new();
    let mut elems: Vec<Elem> = Vec::new();
    let mut nsets: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut elsets: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut materials: HashMap<String, Elastic> = HashMap::new();
    let mut sections: Vec<(String, String, f64)> = Vec::new(); // (elset, material, thickness)
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("**")).peekable();
    let (mut cur_mat, mut cur_alpha): (Option<String>, f64) = (None, 0.0);
    while let Some(line) = lines.next() {
        if !line.starts_with('*') {
            continue;
        }
        let (kw, params, flags) = keyword(line);
        let mut data: Vec<&str> = Vec::new();
        while lines.peek().is_some_and(|l| !l.starts_with('*')) {
            data.push(lines.next().unwrap());
        }
        let tokens = |data: &[&str]| -> Vec<String> { data.iter().flat_map(|l| l.split(',')).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect() };
        match kw.as_str() {
            "NODE" => {
                for l in &data {
                    let t: Vec<&str> = l.split(',').map(str::trim).collect();
                    if t.len() < 3 {
                        return Err(format!("bad node line '{l}'"));
                    }
                    let z = if t.len() > 3 && !t[3].is_empty() { num(t[3])? } else { 0.0 };
                    nodes.push((idx(t[0])?, [num(t[1])?, num(t[2])?, z]));
                }
                if let Some(set) = params.get("NSET") {
                    nsets.entry(set.clone()).or_default().extend(nodes.iter().skip(nodes.len() - data.len()).map(|n| n.0));
                }
            }
            "ELEMENT" => {
                let ty = params.get("TYPE").ok_or("*ELEMENT without TYPE")?;
                let (kind, physics) = abaqus_kind(ty).ok_or_else(|| format!("unsupported Abaqus element type {ty}"))?;
                let toks = tokens(&data);
                let per = 1 + kind.n_nodes();
                if toks.len() % per != 0 {
                    return Err(format!("*ELEMENT, TYPE={ty}: {} values is not a multiple of {per}", toks.len()));
                }
                for chunk in toks.chunks(per) {
                    let ids: Vec<i64> = chunk.iter().map(|t| idx(t)).collect::<Result<_, _>>()?;
                    if let Some(set) = params.get("ELSET") {
                        elsets.entry(set.clone()).or_default().push(ids[0]);
                    }
                    elems.push(Elem { id: ids[0], kind, physics, nodes: ids[1..].to_vec() });
                }
            }
            "NSET" | "ELSET" => {
                let key = if kw == "NSET" { "NSET" } else { "ELSET" };
                let name = params.get(key).ok_or_else(|| format!("*{kw} without {key}"))?.clone();
                let toks = tokens(&data);
                let target = if kw == "NSET" { &mut nsets } else { &mut elsets };
                let mut ids: Vec<i64> = Vec::new();
                if flags.iter().any(|f| f == "GENERATE") {
                    for g in toks.chunks(3) {
                        let (a, b) = (idx(&g[0])?, idx(&g[1])?);
                        let step = g.get(2).map(|s| idx(s)).transpose()?.unwrap_or(1).max(1);
                        ids.extend((a..=b).step_by(step as usize));
                    }
                } else {
                    for t in &toks {
                        match t.parse::<i64>() {
                            Ok(v) => ids.push(v),
                            Err(_) => ids.extend(target.get(t.as_str()).cloned().unwrap_or_default()),
                        }
                    }
                }
                target.entry(name).or_default().extend(ids);
            }
            "MATERIAL" => {
                cur_mat = params.get("NAME").cloned();
                cur_alpha = 0.0;
            }
            "ELASTIC" => {
                let name = cur_mat.clone().ok_or("*ELASTIC outside a *MATERIAL")?;
                let ty = params.get("TYPE").map(|s| s.to_ascii_uppercase()).unwrap_or_else(|| "ISOTROPIC".into());
                let toks = tokens(&data);
                let v: Vec<f64> = toks.iter().map(|t| num(t)).collect::<Result<_, _>>()?;
                let m = match ty.as_str() {
                    "ISOTROPIC" if v.len() >= 2 => Elastic::new(v[0], v[1]),
                    "ENGINEERING CONSTANTS" if v.len() >= 9 => Elastic::orthotropic(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[8], v[7])?,
                    _ => return Err(format!("*ELASTIC, TYPE={ty} with {} values is not supported (isotropic or engineering constants)", v.len())),
                };
                materials.insert(name, Elastic { alpha: cur_alpha, ..m });
            }
            "EXPANSION" => {
                let alpha = tokens(&data).first().map(|t| num(t)).transpose()?.unwrap_or(0.0);
                cur_alpha = alpha;
                if let Some(m) = cur_mat.as_ref().and_then(|n| materials.get_mut(n)) {
                    m.alpha = alpha;
                }
            }
            "SOLID SECTION" => {
                let elset = params.get("ELSET").ok_or("*SOLID SECTION without ELSET")?.clone();
                let mat = params.get("MATERIAL").ok_or("*SOLID SECTION without MATERIAL")?.clone();
                let thickness = tokens(&data).first().map(|t| num(t)).transpose()?.unwrap_or(1.0);
                sections.push((elset, mat, thickness));
            }
            _ => {}
        }
    }
    if elems.is_empty() {
        return Err("the file contains no supported elements".into());
    }
    let dim = elems[0].kind.dim();
    if elems.iter().any(|e| e.kind.dim() != dim) {
        return Err("mixed 2D and 3D elements are not supported".into());
    }
    let physics = if dim == 3 {
        Physics::Solid
    } else {
        let p = elems[0].physics.unwrap_or(opt.physics);
        if elems.iter().any(|e| e.physics.as_ref().map(std::mem::discriminant) != elems[0].physics.as_ref().map(std::mem::discriminant)) {
            return Err("mixed plane stress / plane strain / axisymmetric element types".into());
        }
        // The section thickness applies to the whole (plane) model.
        match p {
            Physics::PlaneStress { .. } => Physics::PlaneStress { thickness: sections.first().map_or(1.0, |s| s.2) },
            Physics::PlaneStrain { .. } => Physics::PlaneStrain { thickness: sections.first().map_or(1.0, |s| s.2) },
            other => other,
        }
    };
    let mut mesh = Mesh::new(physics);
    let mut new_id: HashMap<i64, usize> = HashMap::new();
    let coord: HashMap<i64, [f64; 3]> = nodes.iter().copied().collect();
    let mut elem_set_of: HashMap<i64, &String> = HashMap::new();
    for (name, ids) in &elsets {
        for &i in ids {
            elem_set_of.entry(i).or_insert(name);
        }
    }
    let mat_of_set: HashMap<&str, &str> = sections.iter().map(|s| (s.0.as_str(), s.1.as_str())).collect();
    let mut order: Vec<(String, ElementKind)> = Vec::new();
    let mut conns: HashMap<(String, ElementKind), (Vec<usize>, Elastic)> = HashMap::new();
    for e in &elems {
        let set = elem_set_of.get(&e.id).map(|s| s.to_string()).unwrap_or_else(|| "domain".to_string());
        let mat = match mat_of_set.get(set.as_str()) {
            Some(m) => *materials.get(*m).ok_or_else(|| format!("material '{m}' has no *ELASTIC"))?,
            None => opt.material,
        };
        let entry = conns.entry((set.clone(), e.kind)).or_insert_with(|| {
            order.push((set.clone(), e.kind));
            (Vec::new(), mat)
        });
        for n in &e.nodes {
            let id = match new_id.get(n) {
                Some(&i) => i,
                None => {
                    let c = coord.get(n).ok_or_else(|| format!("element refers to missing node {n}"))?;
                    let i = mesh.add_node(if dim == 2 { [c[0], c[1], 0.0] } else { *c });
                    new_id.insert(*n, i);
                    i
                }
            };
            entry.0.push(id);
        }
    }
    for key in order {
        let (conn, mat) = conns.remove(&key).unwrap();
        mesh.add_block(key.1, conn, mat, &key.0)?;
    }
    check_node_order(&mesh, "Abaqus import")?;
    for (name, ids) in nsets {
        let mut v: Vec<usize> = ids.iter().filter_map(|i| new_id.get(i).copied()).collect();
        v.sort_unstable();
        v.dedup();
        if !v.is_empty() {
            mesh.node_sets.insert(name, v);
        }
    }
    Ok(mesh)
}
