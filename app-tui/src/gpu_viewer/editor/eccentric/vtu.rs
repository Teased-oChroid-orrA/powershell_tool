//! The solved model of an eccentric-bushing analysis as a window scene. The solver (`eccentric-bushing`) hands out its
//! finite-element fields only as ASCII `.vtu` text (`analyze_fields`: the displacement vector, von Mises and the stress
//! components at the nodes of the housing, bushing and pin); this reads that text back into a mesh and the nodal fields the
//! window draws. It reads what `fea_core::vtu::write` writes and nothing more.
use crate::gpu_viewer::scene::SceneSource;
use fea_core::{
    element::ElementKind,
    mesh::{Elastic, Mesh, Physics},
};
use std::{collections::BTreeMap, sync::Arc};

/// One `<DataArray>`: its name (empty for the points), components per entry and numbers.
struct Array {
    name: String,
    ncomp: usize,
    data: Vec<f64>,
}

fn attr<'a>(head: &'a str, key: &str) -> Option<&'a str> {
    let at = head.find(&format!("{key}=\""))? + key.len() + 2;
    head[at..].split('"').next()
}

fn arrays(text: &str) -> Result<Vec<Array>, String> {
    let mut out = Vec::new();
    for part in text.split("<DataArray").skip(1) {
        let (head, rest) = part.split_once('>').ok_or("an unterminated array")?;
        let body = rest.split("</DataArray>").next().ok_or("an unterminated array")?;
        let data = body.split_ascii_whitespace().map(str::parse::<f64>).collect::<Result<Vec<_>, _>>().map_err(|e| format!("a bad number in the fields: {e}"))?;
        out.push(Array { name: attr(head, "Name").unwrap_or("").to_string(), ncomp: attr(head, "NumberOfComponents").and_then(|n| n.parse().ok()).unwrap_or(1), data });
    }
    Ok(out)
}

fn kind_of(vtk: u8) -> Result<ElementKind, String> {
    Ok(match vtk {
        5 => ElementKind::Tri3,
        22 => ElementKind::Tri6,
        9 => ElementKind::Quad4,
        23 => ElementKind::Quad8,
        28 => ElementKind::Quad9,
        other => return Err(format!("cell type {other} is not a plane element")),
    })
}

/// The mesh and fields of a plane `.vtu` as a scene source: von Mises first, then the displacement magnitude and the
/// in-plane stress components. `thickness` is the model's, for the mesh's physics.
pub fn scene_source(text: &str, label: &str, thickness: f64) -> Result<SceneSource, String> {
    let arrays = arrays(text)?;
    let get = |name: &str| arrays.iter().find(|a| a.name == name).ok_or_else(|| format!("the fields have no {name}"));
    let points = arrays.iter().find(|a| a.name.is_empty() && a.ncomp == 3).ok_or("the fields have no points")?;
    let (conn, offsets, types) = (get("connectivity")?, get("offsets")?, get("types")?);
    let (disp, vm, stress) = (get("displacement")?, get("von_mises")?, get("stress_xx_yy_zz_xy_yz_zx")?);
    let n = points.data.len() / 3;
    if n == 0 || disp.data.len() != 3 * n || vm.data.len() != n || stress.data.len() != 6 * n {
        return Err("the fields do not match the points".into());
    }
    let mut mesh = Mesh::new(Physics::PlaneStress { thickness });
    for p in points.data.chunks_exact(3) {
        mesh.add_node([p[0], p[1], 0.0]);
    }
    // Consecutive cells of one type are one block.
    let material = Elastic { e: 1.0, nu: 0.3, alpha: 0.0, aniso: None };
    let (mut start, mut block): (usize, Option<(u8, Vec<usize>)>) = (0, None);
    for (cell, end) in offsets.data.iter().map(|o| *o as usize).enumerate() {
        let ty = *types.data.get(cell).ok_or("a cell without a type")? as u8;
        let nodes = conn.data.get(start..end).ok_or("a cell outside the connectivity")?.iter().map(|v| *v as usize);
        match &mut block {
            Some((t, c)) if *t == ty => c.extend(nodes),
            _ => {
                if let Some((t, c)) = block.take() {
                    mesh.add_block(kind_of(t)?, c, material, "body")?;
                }
                block = Some((ty, nodes.collect()));
            }
        }
        start = end;
    }
    if let Some((t, c)) = block {
        mesh.add_block(kind_of(t)?, c, material, "body")?;
    }
    let displacement: Vec<f64> = disp.data.chunks_exact(3).flat_map(|d| [d[0], d[1]]).collect();
    let magnitude: Vec<f64> = disp.data.chunks_exact(3).map(|d| d[0].hypot(d[1])).collect();
    let comp = |k: usize| stress.data.chunks_exact(6).map(|s| s[k]).collect::<Vec<f64>>();
    let fields: BTreeMap<usize, (&str, Vec<f64>)> = BTreeMap::from([(0, ("Displacement magnitude", magnitude)), (1, ("Stress xx", comp(0))), (2, ("Stress yy", comp(1))), (3, ("Stress xy", comp(3)))]);
    Ok(SceneSource {
        mesh: Arc::new(mesh),
        displacement,
        values: vm.data.clone(),
        period: None,
        label: label.to_string(),
        field_label: "von Mises stress".into(),
        fields: fields.into_values().map(|(l, v)| (l.to_string(), v)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fea_core::vtu::{pad3, write, Field};

    #[test]
    fn what_the_writer_wrote_reads_back_as_the_same_mesh_and_fields() {
        // Two quads and a triangle: a mixed mesh keeps its blocks.
        let mut m = Mesh::new(Physics::PlaneStress { thickness: 0.5 });
        for p in [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, 1.0], [1.0, 2.0]] {
            m.add_node([p[0], p[1], 0.0]);
        }
        let mat = Elastic { e: 1.0, nu: 0.3, alpha: 0.0, aniso: None };
        m.add_block(ElementKind::Quad4, vec![0, 1, 4, 3, 1, 2, 5, 4], mat, "a").unwrap();
        m.add_block(ElementKind::Tri3, vec![3, 4, 6], mat, "b").unwrap();
        let u: Vec<f64> = (0..14).map(|i| i as f64 * 0.01).collect();
        let disp = pad3(&u, 2);
        let vm: Vec<f64> = (0..7).map(|i| 100.0 * (i + 1) as f64).collect();
        let stress: Vec<f64> = (0..42).map(|i| i as f64).collect();
        let text = write(&m, &[Field { name: "displacement", ncomp: 3, data: &disp }, Field { name: "von_mises", ncomp: 1, data: &vm }, Field { name: "stress_xx_yy_zz_xy_yz_zx", ncomp: 6, data: &stress }], &[]).unwrap();
        let s = scene_source(&text, "case", 0.5).unwrap();
        assert_eq!((s.mesh.nodes.len(), s.mesh.n_elems(), s.mesh.blocks.len()), (7, 3, 2));
        assert_eq!(s.mesh.nodes[6], [1.0, 2.0, 0.0]);
        assert_eq!(s.values, vm);
        assert_eq!(s.displacement, u);
        let names: Vec<_> = s.fields.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["Displacement magnitude", "Stress xx", "Stress yy", "Stress xy"]);
        assert!((s.fields[0].1[1] - (0.02f64.hypot(0.03))).abs() < 1e-12);
        assert_eq!(s.fields[3].1[0], 3.0, "the xy component is the fourth of six");
        assert!(s.build().is_ok(), "the window can draw it");
    }

    #[test]
    fn text_that_is_not_the_writers_output_is_refused_with_a_reason() {
        assert!(scene_source("", "x", 1.0).is_err());
        assert!(scene_source("<DataArray Name=\"von_mises\">1 2</DataArray>", "x", 1.0).unwrap_err().contains("points"));
    }
}
