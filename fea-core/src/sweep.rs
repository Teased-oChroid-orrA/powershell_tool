//! Sweeping a 2D quadrilateral mesh into a 3D hexahedral mesh: [`extrude`] along `z` and
//! [`revolve`] about the `y` axis. Quad4 -> Hex8, Quad8 -> Hex20, Quad9 -> Hex27, layer by layer
//! (a layer's mid-plane nodes are placed halfway in the sweep parameter, exactly on the circle for
//! a revolution).
//!
//! Hexahedron node order is VTK's: bottom corners 0-3 (the quad's counter-clockwise order), top
//! corners 4-7 directly above, then (Hex20/27) bottom edge midsides 8-11, top 12-15, vertical edge
//! midsides 16-19, face centres 20-25 (`x-, x+, y-, y+, z-, z+`) and the body centre 26. The sweep
//! direction is `+z` for an extrusion; a revolution maps `(r, z) -> (r cos(phi), z, r sin(phi))`
//! so that `e_r x e_z = e_phi` and the Jacobian stays positive.
//!
//! Node sets of the 2D mesh are carried over (all layers), plus `start` and `end` (first and last
//! layer boundary); 2D named surfaces become the lateral faces, and `start` / `end` surfaces are
//! the end faces (outward-oriented, ready for `Loads::faces`).

use crate::element::ElementKind;
use crate::mesh::{Mesh, Physics};
use std::collections::HashMap;

/// Extrude along `z` through the layer boundaries `z_levels` (strictly increasing, at least two).
pub fn extrude(mesh: &Mesh, z_levels: &[f64]) -> Result<Mesh, String> {
    if matches!(mesh.physics, Physics::Axisymmetric) {
        return Err("an axisymmetric mesh is revolved, not extruded".into());
    }
    sweep(mesh, z_levels, &|x, z| [x[0], x[1], z])
}

/// Revolve about the `y` axis through the angle boundaries `angles` (radians, strictly
/// increasing, at least two). The 2D `x` is the radius and must be non-negative.
pub fn revolve(mesh: &Mesh, angles: &[f64]) -> Result<Mesh, String> {
    if mesh.nodes.iter().any(|x| x[0] < 0.0) {
        return Err("a revolved mesh needs non-negative radii (x >= 0)".into());
    }
    if angles.last().zip(angles.first()).is_some_and(|(b, a)| b - a > std::f64::consts::TAU + 1e-12) {
        return Err("a revolution cannot exceed 360 degrees".into());
    }
    sweep(mesh, angles, &|x, phi| [x[0] * phi.cos(), x[1], x[0] * phi.sin()])
}

/// A sweep that returns to its start (360 degrees).
fn closed_sweep(levels: &[f64]) -> bool {
    (levels[levels.len() - 1] - levels[0] - std::f64::consts::TAU).abs() < 1e-9
}

type SweepMap<'a> = &'a dyn Fn([f64; 3], f64) -> [f64; 3];

fn sweep(mesh: &Mesh, levels: &[f64], pos: SweepMap) -> Result<Mesh, String> {
    if mesh.dim() != 2 {
        return Err("only 2D meshes can be swept".into());
    }
    if levels.len() < 2 || levels.windows(2).any(|w| w[1].is_nan() || w[1] <= w[0]) {
        return Err("sweep levels must be strictly increasing and at least two".into());
    }
    let mut out = Mesh::new(Physics::Solid);
    // 3D node of (2D node, half-step index): even = layer boundary, odd = mid-layer. A closed
    // revolution (360 degrees) identifies the last layer boundary with the first.
    let mut ids: HashMap<(usize, usize), usize> = HashMap::new();
    let n_layers = levels.len() - 1;
    let closed = closed_sweep(levels);
    let mut node = |out: &mut Mesh, n2: usize, half: usize| -> usize {
        let key = if closed && half == 2 * n_layers { 0 } else { half };
        *ids.entry((n2, key)).or_insert_with(|| {
            #[allow(clippy::manual_is_multiple_of)] // is_multiple_of needs a newer toolchain than the CI pin
            let s = if half % 2 == 0 { levels[half / 2] } else { 0.5 * (levels[half / 2] + levels[half / 2 + 1]) };
            out.add_node(pos(mesh.nodes[n2], s))
        })
    };
    for blk in &mesh.blocks {
        let kind3 = match blk.kind {
            ElementKind::Quad4 => ElementKind::Hex8,
            ElementKind::Quad8 => ElementKind::Hex20,
            ElementKind::Quad9 => ElementKind::Hex27,
            k => return Err(format!("{k:?} cannot be swept (no prism elements); mesh with quadrilaterals")),
        };
        let mut conn = Vec::with_capacity(blk.n_elems() * n_layers * kind3.n_nodes());
        for q in blk.conn.chunks_exact(blk.kind.n_nodes()) {
            for layer in 0..n_layers {
                let (lo, mid, hi) = (2 * layer, 2 * layer + 1, 2 * layer + 2);
                for half in [lo, hi] {
                    for &c in &q[..4] {
                        conn.push(node(&mut out, c, half));
                    }
                }
                if kind3 == ElementKind::Hex8 {
                    continue;
                }
                for half in [lo, hi] {
                    for &c in &q[4..8] {
                        conn.push(node(&mut out, c, half));
                    }
                }
                for &c in &q[..4] {
                    conn.push(node(&mut out, c, mid));
                }
                if kind3 == ElementKind::Hex27 {
                    // Face centres x-, x+, y-, y+ sit at the mid-plane of the matching 2D edge
                    // midside (3-0, 1-2, 0-1, 2-3), then z- / z+ at the quad centre, then the body centre.
                    for &c in &[q[7], q[5], q[4], q[6]] {
                        conn.push(node(&mut out, c, mid));
                    }
                    conn.push(node(&mut out, q[8], lo));
                    conn.push(node(&mut out, q[8], hi));
                    conn.push(node(&mut out, q[8], mid));
                }
            }
        }
        out.add_block(kind3, conn, blk.material, &blk.name)?;
    }
    // Node sets: every layer of the 2D set; plus the two end layers.
    for (name, set) in &mesh.node_sets {
        let mut v: Vec<usize> = set.iter().flat_map(|&n2| (0..=2 * n_layers).filter_map(|h| ids.get(&(n2, h)).copied()).collect::<Vec<_>>()).collect();
        v.sort_unstable();
        v.dedup();
        out.node_sets.insert(name.clone(), v);
    }
    for (name, half) in [("start", 0), ("end", if closed { 0 } else { 2 * n_layers })] {
        let mut v: Vec<usize> = ids.iter().filter(|(k, _)| k.1 == half).map(|(_, &v)| v).collect();
        v.sort_unstable();
        out.node_sets.insert(name.to_string(), v);
    }
    // Surfaces from the boundary faces: end faces are single-level, lateral faces project onto a 2D surface.
    let parent: HashMap<usize, (usize, usize)> = ids.iter().map(|(&(n2, h), &n3)| (n3, (n2, h))).collect();
    let faces = out.boundary_faces();
    for (name, half) in [("start", 0), ("end", 2 * n_layers)] {
        if closed {
            out.surfaces.insert(name.to_string(), Vec::new());
            continue;
        }
        let f: Vec<Vec<usize>> = faces.iter().filter(|f| f.nodes.iter().all(|n| parent[n].1 == half)).map(|f| f.nodes.clone()).collect();
        out.surfaces.insert(name.to_string(), f);
    }
    for (name, edges) in &mesh.surfaces {
        let on: std::collections::HashSet<usize> = edges.iter().flatten().copied().collect();
        let f: Vec<Vec<usize>> = faces
            .iter()
            .filter(|f| {
                let levels_hit: std::collections::HashSet<usize> = f.nodes.iter().map(|n| parent[n].1).collect();
                levels_hit.len() > 1 && f.nodes.iter().all(|n| on.contains(&parent[n].0))
            })
            .map(|f| f.nodes.clone())
            .collect();
        out.surfaces.insert(name.clone(), f);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

    use super::*;
    use crate::generate::grid;
    use crate::mesh::Elastic;

    fn plate(kind: ElementKind) -> Mesh {
        grid(Physics::PlaneStrain { thickness: 1.0 }, kind, Elastic::new(1.0, 0.3), [2, 2, 1], &|p| [2.0 * p[0] + 0.2 * p[1], 1.5 * p[1], 0.0]).unwrap()
    }

    /// Every node of every hexahedron sits where the (straight-sided) element map puts it: this
    /// pins the whole node ordering, midsides and face centres included.
    fn assert_straight_sided(m: &Mesh) {
        for blk in &m.blocks {
            let nc = blk.kind.n_corners();
            let (kind, nc_coords) = (blk.kind, blk.kind.node_coords());
            let lin = if kind == ElementKind::Hex8 { kind } else { ElementKind::Hex8 };
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                for (k, xi) in nc_coords.iter().enumerate() {
                    let (n, _) = lin.shape(*xi);
                    let mut want = [0.0; 3];
                    for a in 0..nc {
                        for i in 0..3 {
                            want[i] += n[a] * m.nodes[conn[a]][i];
                        }
                    }
                    let got = m.nodes[conn[k]];
                    assert!((0..3).all(|i| (got[i] - want[i]).abs() < 1e-12), "{kind:?} element {e} node {k}: {got:?} vs {want:?}");
                }
            }
        }
    }

    #[test]
    fn extruded_nodes_have_the_vtk_hexahedron_order_and_positive_volume() {
        for (q, h) in [(ElementKind::Quad4, ElementKind::Hex8), (ElementKind::Quad8, ElementKind::Hex20), (ElementKind::Quad9, ElementKind::Hex27)] {
            let m2 = plate(q);
            let m3 = extrude(&m2, &[0.0, 0.5, 1.5, 2.0]).unwrap();
            assert_eq!(m3.blocks[0].kind, h);
            assert_eq!(m3.n_elems(), m2.n_elems() * 3);
            assert_straight_sided(&m3);
            // Volume = plane area x height (Jacobian positive everywhere via the stiffness assembly).
            let model = crate::Model::new(m3.clone()).unwrap();
            model.assemble().expect("positive Jacobians");
            // Shared nodes are shared: Hex27 has (2*nx+1)(2*ny+1)(2*nz+1) nodes.
            if h == ElementKind::Hex27 {
                assert_eq!(m3.nodes.len(), 5 * 5 * 7);
            }
            if h == ElementKind::Hex8 {
                assert_eq!(m3.nodes.len(), 3 * 3 * 4);
            }
        }
    }

    #[test]
    fn extrusion_carries_sets_and_builds_end_and_lateral_surfaces() {
        let m2 = plate(ElementKind::Quad9);
        let m3 = extrude(&m2, &[0.0, 1.0, 2.0]).unwrap();
        assert_eq!(m3.node_sets["start"].len(), m2.nodes.len());
        assert!(m3.node_sets["start"].iter().all(|&n| m3.nodes[n][2] == 0.0) && m3.node_sets["end"].iter().all(|&n| m3.nodes[n][2] == 2.0));
        // 2D set u0 (left edge, 5 nodes) over 5 z half-levels.
        assert_eq!(m3.node_sets["u0"].len(), 5 * 5);
        // End surfaces: 4 quad faces each, of nine nodes, outward normal -z at start and +z at end.
        assert_eq!(m3.surfaces["start"].len(), 4);
        assert!(m3.surfaces["start"].iter().all(|f| f.len() == 9));
        use crate::loads::{assemble, Loads, SurfaceLoad};
        for (name, sign) in [("start", -1.0), ("end", 1.0)] {
            let loads = Loads { faces: m3.surfaces[name].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(1.0))).collect(), ..Loads::default() };
            let f = assemble(&m3, &loads).unwrap();
            let fz: f64 = (0..m3.nodes.len()).map(|n| f[n * 3 + 2]).sum();
            // Pressure pushes into the body: on the end face (normal +z) the force is -p A in z.
            let area = 2.0 * 1.5; // plane area of the plate map (det = 2 * 1.5)
            assert!((fz + sign * area).abs() < 1e-12, "{name}: {fz}");
        }
        // Lateral surface u0: 2 elements x 2 layers = 4 faces; normal -x-ish; area = edge length x height.
        assert_eq!(m3.surfaces["u0"].len(), 4);
    }

    #[test]
    fn invalid_sweeps_are_rejected() {
        let m2 = plate(ElementKind::Quad9);
        assert!(extrude(&m2, &[0.0]).is_err() && extrude(&m2, &[0.0, 1.0, 1.0]).is_err());
        let tri = grid(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6, Elastic::new(1.0, 0.3), [1, 1, 1], &|p| p).unwrap();
        assert!(extrude(&tri, &[0.0, 1.0]).unwrap_err().contains("prism"));
        let ax = grid(Physics::Axisymmetric, ElementKind::Quad9, Elastic::new(1.0, 0.3), [1, 1, 1], &|p| [1.0 + p[0], p[1], 0.0]).unwrap();
        assert!(extrude(&ax, &[0.0, 1.0]).is_err());
        assert!(revolve(&ax, &[0.0, 7.0]).is_err());
    }
}
