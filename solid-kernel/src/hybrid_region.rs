//! The hexahedral core of a body: choose the kept hexahedral region of a closed shell and join it to the tetrahedra that fill
//! the rest ([`crate::hybrid::hex_pyramid_tet`]).
//!
//! The core is a union of the cells of a regular grid. A cell is kept when all eight of its corners lie inside the body, at
//! least `margin_cells` cell edges from the surface (room for the pyramid layer and for the tetrahedra between the pyramids and
//! the shell). Of the kept cells only the largest face-connected group stays, and cells are dropped until the boundary of the
//! group is a closed manifold without pinches (an edge or a vertex where two parts of the boundary only touch), so that every
//! interface quadrilateral has a pyramid and the pyramid layer is a closed surface. That surface is a **cavity** of the
//! tetrahedral region: `conform_preserving` fills the space between the shell and the pyramid sides.
//!
//! Only an interior core is built (it never touches the shell); a core that reaches the surface (a hexahedral end of a part)
//! needs the shell and the hexahedra to share their boundary ring, which is `hex_pyramid_tet`'s own contract.
//! `docs/issue-46-phase-111.md` has the evidence and the limits.

use crate::hybrid::{hex_pyramid_tet, HybridOptions, HybridReport};
use crate::{SurfaceMesh, SurfaceSolid, V3};
use fea_core::{Elastic, ElementKind, Mesh, Physics};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy)]
pub struct RegionOptions {
    /// Edge length of the hexahedra (mm).
    pub cell: f64,
    /// Smallest distance of a kept cell's corners from the surface, in cell edges. The pyramids reach `hybrid.apex_height`
    /// cell edges beyond the core, so this must exceed it with room for tetrahedra.
    pub margin_cells: f64,
    pub hybrid: HybridOptions,
}

impl Default for RegionOptions {
    fn default() -> Self {
        Self { cell: 1.0, margin_cells: 2.0, hybrid: HybridOptions::default() }
    }
}

type Cell = [usize; 3];

/// The cells of a grid over `lo`/`hi` of edge `h` whose corners all lie at least `margin` inside the body.
fn inside_cells(solid: &SurfaceSolid, lo: V3, hi: V3, h: f64, margin: f64) -> BTreeSet<Cell> {
    let n: [usize; 3] = std::array::from_fn(|k| ((hi[k] - lo[k]) / h).ceil().max(1.0) as usize);
    let corner = |i: usize, j: usize, k: usize| [lo[0] + i as f64 * h, lo[1] + j as f64 * h, lo[2] + k as f64 * h];
    let id = |i: usize, j: usize, k: usize| (k * (n[1] + 1) + j) * (n[0] + 1) + i;
    let mut deep = vec![false; (n[0] + 1) * (n[1] + 1) * (n[2] + 1)];
    for k in 0..=n[2] {
        for j in 0..=n[1] {
            for i in 0..=n[0] {
                deep[id(i, j, k)] = solid.signed_distance(corner(i, j, k)) <= -margin;
            }
        }
    }
    let mut cells = BTreeSet::new();
    for k in 0..n[2] {
        for j in 0..n[1] {
            for i in 0..n[0] {
                let all = (0..8).all(|c| deep[id(i + (c & 1), j + ((c >> 1) & 1), k + ((c >> 2) & 1))]);
                if all {
                    cells.insert([i, j, k]);
                }
            }
        }
    }
    cells
}

fn face_neighbours(c: Cell) -> [(Cell, usize, bool); 6] {
    let mut out = [([0; 3], 0, false); 6];
    for (m, axis) in (0..3).enumerate() {
        for (s, up) in [false, true].into_iter().enumerate() {
            let mut q = c;
            if up {
                q[axis] += 1;
            } else if q[axis] == 0 {
                q[axis] = usize::MAX; // outside the grid: never a kept cell
            } else {
                q[axis] -= 1;
            }
            out[2 * m + s] = (q, axis, up);
        }
    }
    out
}

/// The largest face-connected group of `cells`.
fn largest_group(cells: &BTreeSet<Cell>) -> BTreeSet<Cell> {
    let mut seen: BTreeSet<Cell> = BTreeSet::new();
    let mut best: Vec<Cell> = Vec::new();
    for &start in cells {
        if seen.contains(&start) {
            continue;
        }
        let mut group = vec![start];
        let mut queue = VecDeque::from([start]);
        seen.insert(start);
        while let Some(c) = queue.pop_front() {
            for (q, _, _) in face_neighbours(c) {
                if cells.contains(&q) && seen.insert(q) {
                    group.push(q);
                    queue.push_back(q);
                }
            }
        }
        if group.len() > best.len() {
            best = group;
        }
    }
    best.into_iter().collect()
}

/// Grid corner (i, j, k) as a key, and the corners of a cell's face (axis, up) in counter-clockwise order seen from outside the
/// cell.
fn face_corners(c: Cell, axis: usize, up: bool) -> [[usize; 3]; 4] {
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let base = |da: usize, db: usize| {
        let mut p = c;
        if up {
            p[axis] += 1;
        }
        p[a] += da;
        p[b] += db;
        p
    };
    // (a, b, axis) is a right-handed frame, so a ccw walk in the (a, b) plane looks along -axis from `up`; reverse for the low side
    let q = [base(0, 0), base(1, 0), base(1, 1), base(0, 1)];
    if up {
        q
    } else {
        [q[0], q[3], q[2], q[1]]
    }
}

/// A cell whose removal fixes a pinch of the group's boundary, if there is one: an edge shared by more than two boundary
/// quadrilaterals, or a vertex whose boundary quadrilaterals fall into more than one edge-connected fan.
fn pinched_cell(cells: &BTreeSet<Cell>) -> Option<Cell> {
    let mut edge_quads: BTreeMap<([usize; 3], [usize; 3]), Vec<(Cell, usize)>> = BTreeMap::new();
    let mut vertex_quads: BTreeMap<[usize; 3], Vec<(Cell, usize)>> = BTreeMap::new();
    for &c in cells {
        for (f, (q, axis, up)) in face_neighbours(c).into_iter().enumerate() {
            if cells.contains(&q) {
                continue;
            }
            let corners = face_corners(c, axis, up);
            for k in 0..4 {
                let (a, b) = (corners[k], corners[(k + 1) % 4]);
                edge_quads.entry((a.min(b), a.max(b))).or_default().push((c, f));
                vertex_quads.entry(a).or_default().push((c, f));
            }
        }
    }
    let neighbours = |c: Cell| face_neighbours(c).iter().filter(|(q, _, _)| cells.contains(q)).count();
    let weakest = |owners: &[(Cell, usize)]| owners.iter().map(|o| o.0).min_by_key(|&c| (neighbours(c), c)).unwrap();
    for owners in edge_quads.values() {
        if owners.len() != 2 {
            return Some(weakest(owners));
        }
    }
    for (v, owners) in &vertex_quads {
        // quads sharing an edge at this vertex are one fan: union over the edges incident to `v`
        let mut parent: Vec<usize> = (0..owners.len()).collect();
        fn find(p: &mut Vec<usize>, x: usize) -> usize {
            if p[x] != x {
                let r = find(p, p[x]);
                p[x] = r;
            }
            p[x]
        }
        let mut by_edge: BTreeMap<[usize; 3], usize> = BTreeMap::new();
        for (qi, &(c, f)) in owners.iter().enumerate() {
            let (_, axis, up) = face_neighbours(c)[f];
            let corners = face_corners(c, axis, up);
            let at = corners.iter().position(|x| x == v).unwrap();
            for other in [corners[(at + 1) % 4], corners[(at + 3) % 4]] {
                match by_edge.get(&other) {
                    Some(&first) => {
                        let (ra, rb) = (find(&mut parent, first), find(&mut parent, qi));
                        parent[ra] = rb;
                    }
                    None => {
                        by_edge.insert(other, qi);
                    }
                }
            }
        }
        let roots: BTreeSet<usize> = (0..owners.len()).map(|i| find(&mut parent, i)).collect();
        if roots.len() > 1 {
            return Some(weakest(owners));
        }
    }
    None
}

/// The kept cells of a grid of edge `h` over the body: deep enough inside, one face-connected group, no boundary pinch.
pub fn core_cells(solid: &SurfaceSolid, h: f64, margin_cells: f64) -> Result<(V3, BTreeSet<Cell>), String> {
    if !(h.is_finite() && h > 0.0 && margin_cells.is_finite() && margin_cells >= 0.0) {
        return Err("the cell size must be positive and the margin not negative".into());
    }
    // a grid anchored at the bounding box's low corner
    let lo = solid.lo;
    let mut cells = inside_cells(solid, lo, solid.hi, h, margin_cells * h);
    loop {
        cells = largest_group(&cells);
        match pinched_cell(&cells) {
            Some(c) => {
                cells.remove(&c);
            }
            None => break,
        }
        if cells.is_empty() {
            break;
        }
    }
    if cells.is_empty() {
        return Err(format!("no hexahedral core fits: no cell of edge {h} lies {margin_cells} edges inside the body"));
    }
    Ok((lo, cells))
}

/// A hexahedral core of cells of edge `options.cell` inside the closed outward `shell` (a native sampled surface), the
/// pyramid layer around it and tetrahedra for the rest of the body, as one face-conforming mesh.
pub fn hex_core_hybrid(shell: &SurfaceMesh, material: Elastic, options: RegionOptions) -> Result<(Mesh, HybridReport), String> {
    let solid = SurfaceSolid::new(shell.clone(), 1e-9 * options.cell.max(1e-3))?;
    let (lo, cells) = core_cells(&solid, options.cell, options.margin_cells)?;
    let h = options.cell;
    let mut hex = Mesh::new(Physics::Solid);
    let mut node_of: BTreeMap<[usize; 3], usize> = BTreeMap::new();
    let mut conn = Vec::with_capacity(8 * cells.len());
    for &c in &cells {
        for corner in 0..8 {
            // the standard Hex8 order: the low face counter-clockwise, then the high face
            let (dx, dy, dz) = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0), (0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)][corner];
            let key = [c[0] + dx, c[1] + dy, c[2] + dz];
            let id = *node_of.entry(key).or_insert_with(|| hex.add_node([lo[0] + key[0] as f64 * h, lo[1] + key[1] as f64 * h, lo[2] + key[2] as f64 * h]));
            conn.push(id);
        }
    }
    hex.add_block(ElementKind::Hex8, conn, material, "hexahedra")?;
    let interface: Vec<[usize; 4]> = hex.boundary_faces().iter().map(|f| [f.nodes[0], f.nodes[1], f.nodes[2], f.nodes[3]]).collect();
    hex_pyramid_tet(&hex, &interface, solid.vertices(), solid.triangles(), options.hybrid)
}
