//! VTK XML unstructured-grid (`.vtu`) writer. A pure function returning the document text; the
//! caller does the file I/O. Node order of every element type is already VTK order.

use crate::mesh::Mesh;
use std::fmt::Write;

/// A named per-node or per-cell field with `ncomp` interleaved components.
pub struct Field<'a> {
    pub name: &'a str,
    pub ncomp: usize,
    pub data: &'a [f64],
}

/// Pad a displacement-like field of `d` components per node to three (VTK vectors are 3D).
pub fn pad3(data: &[f64], d: usize) -> Vec<f64> {
    data.chunks_exact(d).flat_map(|c| (0..3).map(move |i| if i < d { c[i] } else { 0.0 })).collect()
}

/// ASCII `.vtu` text of the mesh with node (`point`) and per-element (`cell`) fields. Cells are in
/// block order. Errors if a field's length does not match the mesh.
pub fn write(mesh: &Mesh, point: &[Field], cell: &[Field]) -> Result<String, String> {
    let n_nodes = mesh.nodes.len();
    let n_cells = mesh.n_elems();
    for (f, n, what) in point.iter().map(|f| (f, n_nodes, "node")).chain(cell.iter().map(|f| (f, n_cells, "element"))) {
        if f.ncomp == 0 || f.data.len() != n * f.ncomp {
            return Err(format!("field '{}' has {} values, expected {} per {what} x {}", f.name, f.data.len(), n, f.ncomp));
        }
    }
    let mut s = String::with_capacity(64 * (n_nodes + n_cells));
    s.push_str("<?xml version=\"1.0\"?>\n<VTKFile type=\"UnstructuredGrid\" version=\"1.0\" byte_order=\"LittleEndian\">\n<UnstructuredGrid>\n");
    let _ = writeln!(s, "<Piece NumberOfPoints=\"{n_nodes}\" NumberOfCells=\"{n_cells}\">");
    s.push_str("<Points>\n<DataArray type=\"Float64\" NumberOfComponents=\"3\" format=\"ascii\">\n");
    for x in &mesh.nodes {
        let z = if mesh.dim() == 3 { x[2] } else { 0.0 };
        let _ = writeln!(s, "{:e} {:e} {:e}", x[0], x[1], z);
    }
    s.push_str("</DataArray>\n</Points>\n<Cells>\n<DataArray type=\"Int64\" Name=\"connectivity\" format=\"ascii\">\n");
    for blk in &mesh.blocks {
        for c in blk.conn.chunks_exact(blk.kind.n_nodes()) {
            let line: Vec<String> = c.iter().map(|n| n.to_string()).collect();
            let _ = writeln!(s, "{}", line.join(" "));
        }
    }
    s.push_str("</DataArray>\n<DataArray type=\"Int64\" Name=\"offsets\" format=\"ascii\">\n");
    let mut off = 0usize;
    for blk in &mesh.blocks {
        for _ in 0..blk.n_elems() {
            off += blk.kind.n_nodes();
            let _ = writeln!(s, "{off}");
        }
    }
    s.push_str("</DataArray>\n<DataArray type=\"UInt8\" Name=\"types\" format=\"ascii\">\n");
    for blk in &mesh.blocks {
        for _ in 0..blk.n_elems() {
            let _ = writeln!(s, "{}", blk.kind.vtk_type());
        }
    }
    s.push_str("</DataArray>\n</Cells>\n");
    for (tag, fields) in [("PointData", point), ("CellData", cell)] {
        if fields.is_empty() {
            continue;
        }
        let _ = writeln!(s, "<{tag}>");
        for f in fields {
            let _ = writeln!(s, "<DataArray type=\"Float64\" Name=\"{}\" NumberOfComponents=\"{}\" format=\"ascii\">", xml_escape(f.name), f.ncomp);
            for row in f.data.chunks_exact(f.ncomp) {
                let line: Vec<String> = row.iter().map(|v| format!("{v:e}")).collect();
                let _ = writeln!(s, "{}", line.join(" "));
            }
            s.push_str("</DataArray>\n");
        }
        let _ = writeln!(s, "</{tag}>");
    }
    s.push_str("</Piece>\n</UnstructuredGrid>\n</VTKFile>\n");
    Ok(s)
}

fn xml_escape(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    #[test]
    fn document_lists_every_point_cell_offset_and_type() {
        let m = grid(Physics::Solid, ElementKind::Hex20, Elastic::new(1.0, 0.3), [2, 1, 1], &|p| p).unwrap();
        let u: Vec<f64> = (0..m.n_dofs()).map(|i| i as f64).collect();
        let s = write(&m, &[Field { name: "u<x>", ncomp: 3, data: &u }], &[Field { name: "id", ncomp: 1, data: &[0.0, 1.0] }]).unwrap();
        assert!(s.contains(&format!("NumberOfPoints=\"{}\" NumberOfCells=\"2\"", m.nodes.len())));
        assert!(s.contains("Name=\"u&lt;x&gt;\" NumberOfComponents=\"3\""));
        // offsets 20, 40 and two cells of VTK type 25
        assert!(s.contains("\n20\n40\n"));
        assert_eq!(s.lines().filter(|l| *l == "25").count(), 2);
        // Connectivity lines parse back to the original element.
        let conn_line = s.split("Name=\"connectivity\" format=\"ascii\">\n").nth(1).unwrap().lines().next().unwrap();
        let got: Vec<usize> = conn_line.split(' ').map(|t| t.parse().unwrap()).collect();
        assert_eq!(got, m.blocks[0].elem(0));
    }

    #[test]
    fn a_wrong_length_field_is_rejected_and_2d_vectors_pad_to_three() {
        let m = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.3), [1, 1, 1], &|p| p).unwrap();
        assert!(write(&m, &[Field { name: "bad", ncomp: 3, data: &[0.0; 5] }], &[]).is_err());
        assert_eq!(pad3(&[1.0, 2.0, 3.0, 4.0], 2), vec![1.0, 2.0, 0.0, 3.0, 4.0, 0.0]);
    }
}
