//! A hexahedral region joined to a tetrahedral one through a layer of pyramids, face-conforming.
//!
//! The kept hexahedra end in quadrilateral faces (the *interface*). Each interface quadrilateral carries a `Pyramid5` whose
//! base is that quadrilateral (so it shares the hexahedron's face exactly) and whose apex lies a little way into the
//! tetrahedral region; the pyramid's four triangular sides are the only faces the tetrahedra see. They are given to
//! [`conform_preserving`] as **frozen facets** together with the outer surface of the tetrahedral region, so the tetrahedra
//! fill exactly what is left and every pyramid side comes out as given. The three element families then share their faces:
//! no tie, no constraint equations, no hanging nodes.
//!
//! `docs/issue-46-phase-102.md` has the evidence and the limits.

use crate::plc_conform::{conform_preserving, Options as ConformOptions};
use crate::V3;
use fea_core::{Elastic, ElementKind, Mesh};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct HybridOptions {
    /// Apex height above the interface quadrilateral, in units of its mean edge length. 0.5 inclines the sides by 45 degrees
    /// on a square (a neighbour's valley is then 90 degrees); lower is flatter, higher reaches deeper into the region.
    pub apex_height: f64,
    /// Settings of the tetrahedral recovery. The default asks for the economical recipe (recover only what is missing) and a
    /// radius-edge ratio of 2.
    pub conform: ConformOptions,
    /// Distance, relative to the shortest interface edge, within which a vertex of the tetrahedral region's surface is the
    /// interface node of the hexahedral mesh.
    pub match_tolerance: f64,
}

impl Default for HybridOptions {
    fn default() -> Self {
        Self {
            apex_height: 0.5,
            conform: ConformOptions {
                max_steiner: 200_000,
                protect_encroached: false,
                max_radius_edge: Some(2.0),
                ..ConformOptions::default()
            },
            match_tolerance: 1e-6,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HybridReport {
    pub hexahedra: usize,
    pub pyramids: usize,
    pub tetrahedra: usize,
    /// Steiner points the tetrahedral recovery added (none on a pyramid side).
    pub steiner_points: usize,
    /// Cells of the tetrahedral region above the radius-edge bound that could not be improved without splitting a side.
    pub unresolved_cells: usize,
    pub max_radius_edge_ratio: f64,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn norm(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The hexahedral mesh `hex` (any blocks of `Hex8`; its node sets and surfaces are kept), its interface quadrilaterals
/// (node lists counter-clockwise seen from outside the hexahedra, i.e. from the tetrahedral side, as
/// `Mesh::boundary_faces` gives them) and the outer surface of the tetrahedral region (`shell_vertices`, `shell_facets`,
/// outward). The shell must close along the interface's boundary edges, whose end points are matched to the interface nodes by
/// position.
///
/// Returns the combined mesh (the hexahedral blocks as they were, a `pyramids` block, a `tetrahedra` block with the material of
/// the first hexahedral block) and a report.
pub fn hex_pyramid_tet(
    hex: &Mesh,
    interface: &[[usize; 4]],
    shell_vertices: &[V3],
    shell_facets: &[[usize; 3]],
    options: HybridOptions,
) -> Result<(Mesh, HybridReport), String> {
    if hex.dim() != 3 || hex.blocks.is_empty() {
        return Err("a hexahedral mesh is needed".into());
    }
    if interface.is_empty() {
        return Err("the interface has no quadrilateral".into());
    }
    if hex.blocks.iter().any(|b| b.kind != ElementKind::Hex8) {
        return Err("only Hex8 blocks can end in an interface".into());
    }
    let material: Elastic = hex.blocks[0].material;
    // typical interface edge length
    let mut edge_sum = 0.0;
    let mut shortest = f64::INFINITY;
    for q in interface {
        for k in 0..4 {
            let l = norm(sub(hex.nodes[q[(k + 1) % 4]], hex.nodes[q[k]]));
            edge_sum += l;
            shortest = shortest.min(l);
        }
    }
    let tol = options.match_tolerance * shortest;
    // shell vertex -> hex node, by position
    let interface_nodes: Vec<usize> = {
        let mut v: Vec<usize> = interface.iter().flatten().copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    // the PLC: shell vertices, then the interface nodes the shell does not contain (the interior ones, which only pyramid sides
    // touch), then one apex per interface quadrilateral; shell facets, then the four frozen sides of each pyramid
    let mut vertices: Vec<V3> = shell_vertices.to_vec();
    let mut matched: BTreeMap<usize, usize> = BTreeMap::new(); // hex node -> PLC vertex
    for &n in &interface_nodes {
        let p = hex.nodes[n];
        let found: Vec<usize> = (0..shell_vertices.len())
            .filter(|&i| norm(sub(shell_vertices[i], p)) <= tol)
            .collect();
        match found.as_slice() {
            [i] => {
                matched.insert(n, *i);
            }
            [] => {
                matched.insert(n, vertices.len());
                vertices.push(p);
            }
            _ => {
                return Err(format!(
                    "interface node {n} matches {} surface vertices",
                    found.len()
                ))
            }
        }
    }
    let mut facets: Vec<[usize; 3]> = shell_facets.to_vec();
    let mut frozen = vec![false; facets.len()];
    let mean_edge = edge_sum / (4 * interface.len()) as f64;
    let mut pyramids: Vec<[usize; 5]> = Vec::with_capacity(interface.len());
    let mut apex_of_pyramid: Vec<usize> = Vec::new(); // PLC vertex of each apex
    let mut apex_at: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    for q in interface {
        let p: Vec<V3> = q.iter().map(|&n| hex.nodes[n]).collect();
        // Newell normal (counter-clockwise from outside: points to the tetrahedral side)
        let mut normal = [0.0; 3];
        for k in 0..4 {
            let (a, b) = (p[k], p[(k + 1) % 4]);
            normal = add(normal, cross(a, b));
        }
        let nl = norm(normal);
        if nl == 0.0 {
            return Err("a degenerate interface quadrilateral".into());
        }
        let centre = mul(add(add(p[0], p[1]), add(p[2], p[3])), 0.25);
        let local_edge = (0..4).map(|k| norm(sub(p[(k + 1) % 4], p[k]))).sum::<f64>() / 4.0;
        let height = options.apex_height
            * if local_edge > 0.0 {
                local_edge
            } else {
                mean_edge
            };
        let apex = add(centre, mul(normal, height / nl));
        // At a concave step of the interface the apexes of the two pyramids over it coincide (their sides meet at a right angle
        // when the apex height is 0.5): they share one vertex.
        let key = apex.map(|x| (x / (1e-6 * shortest)).round() as i64);
        let a = *apex_at.entry(key).or_insert_with(|| {
            vertices.push(apex);
            vertices.len() - 1
        });
        apex_of_pyramid.push(a);
        let s: Vec<usize> = q.iter().map(|n| matched[n]).collect();
        for k in 0..4 {
            // seen from the tetrahedral region the side's normal must point into the pyramid: reverse of the pyramid's outward one
            facets.push([s[(k + 1) % 4], s[k], a]);
            frozen.push(true);
        }
        pyramids.push([q[0], q[1], q[2], q[3], 0]);
    }
    // Two pyramids that share an apex also share the side over their common concave edge: the same triangle seen from both, an
    // interior face of the pyramid layer, not a facet of the tetrahedral region.
    {
        let key = |f: &[usize; 3]| {
            let mut k = *f;
            k.sort_unstable();
            k
        };
        let mut count: BTreeMap<[usize; 3], usize> = BTreeMap::new();
        for (f, &fr) in facets.iter().zip(&frozen) {
            if fr {
                *count.entry(key(f)).or_default() += 1;
            }
        }
        let keep: Vec<bool> = facets.iter().zip(&frozen).map(|(f, &fr)| !(fr && count[&key(f)] == 2)).collect();
        let mut it = keep.iter();
        facets.retain(|_| *it.next().unwrap());
        let mut it = keep.iter();
        frozen.retain(|_| *it.next().unwrap());
    }
    let tets = conform_preserving(&vertices, &facets, &frozen, options.conform, |_| true)
        .map_err(|e| format!("tetrahedral region: {e}"))?;
    // node numbering: the hexahedral nodes keep theirs; every other point of the tetrahedral recovery is new
    let mut out = hex.clone();
    let mut node_of_point: Vec<usize> = vec![usize::MAX; tets.points.len()];
    for (&n, &i) in &matched {
        node_of_point[i] = n;
    }
    for (i, p) in tets.points.iter().enumerate() {
        if node_of_point[i] == usize::MAX {
            node_of_point[i] = out.add_node(*p);
        }
    }
    for (k, py) in pyramids.iter_mut().enumerate() {
        py[4] = node_of_point[apex_of_pyramid[k]];
    }
    out.add_block(
        ElementKind::Pyramid5,
        pyramids.iter().flatten().copied().collect(),
        material,
        "pyramids",
    )?;
    let mut conn = Vec::with_capacity(4 * tets.cells.len());
    for c in &tets.cells {
        conn.extend(c.iter().map(|&i| node_of_point[i]));
    }
    out.add_block(ElementKind::Tet4, conn, material, "tetrahedra")?;
    // the tetrahedra's outer faces are boundary faces like the hexahedra's: nothing to merge, the nodes are shared
    let used = {
        let mut u = vec![false; out.nodes.len()];
        for b in &out.blocks {
            for &n in &b.conn {
                u[n] = true;
            }
        }
        u
    };
    if let Some(n) = used.iter().position(|u| !*u) {
        return Err(format!("node {n} at {:?} belongs to no element ({} hexahedral nodes)", out.nodes[n], hex.nodes.len()));
    }
    let report = HybridReport {
        hexahedra: hex.blocks.iter().map(|b| b.n_elems()).sum(),
        pyramids: pyramids.len(),
        tetrahedra: tets.cells.len(),
        steiner_points: tets.report.segment_steiner
            + tets.report.facet_steiner
            + tets.report.quality_steiner,
        unresolved_cells: tets.report.unresolved_bad_cells,
        max_radius_edge_ratio: tets.report.max_radius_edge_ratio,
    };
    Ok((out, report))
}
