//! Hybrid structured/unstructured meshing of a planar region.
//!
//! [`classify_2d`](super::partition::classify_2d) splits the region into regular rectangles and the cells around features.
//! The regular cells become structured Quad blocks whose divisions per coarse row and column are chosen from the size
//! function; the union of the other cells (bounded by the original outer boundary and by the structured blocks) is meshed by
//! constrained Delaunay with every structured-side edge pre-split into exactly the structured divisions. Both sides place
//! their interface nodes at the same positions, a weld makes them the same nodes, and the result is checked for conformity
//! (a shared edge has the same nodes on both sides), element quality and area before it is returned. Anything the construction
//! cannot guarantee falls back, explicitly, to the plain unstructured mesh of the whole region.
//!
//! Interface compatibility is by construction: a Quad4 edge (2 nodes) meets a Tri3 edge, a Quad8 edge (3 nodes) meets a Tri6
//! edge, both with identical vertex positions, so the displacement trace on the shared edge is the same polynomial from both
//! sides (C0 continuity, no constraint equations). Mixing orders is not supported.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use super::adjacency::{check_conformity, Adjacency, ConformityReport};
use super::partition::{classify_2d, CellGrid, CellState, PartitionOptions2d};
use super::quality::{QualityReport, QualityThresholds};
use super::weld::merge;
use super::{ElementLabels, MeshError, RegionKind};
use crate::delaunay::MeshOptions;
use crate::element::ElementKind;
use crate::geometry::{Curve, Loop, Region, Segment};
use crate::mesh::{Mesh, Physics};
use crate::mesh2d::mesh_region;
use std::collections::HashMap;

/// Element order of the whole mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementFamily {
    /// Quad4 + Tri3.
    Linear,
    /// Quad8 + Tri6 (recommended: Tri3 is poor on pressure-driven problems).
    Quadratic,
}

impl ElementFamily {
    fn quad(self) -> ElementKind {
        match self {
            Self::Linear => ElementKind::Quad4,
            Self::Quadratic => ElementKind::Quad8,
        }
    }
    fn tri(self) -> ElementKind {
        match self {
            Self::Linear => ElementKind::Tri3,
            Self::Quadratic => ElementKind::Tri6,
        }
    }
    fn order(self) -> usize {
        match self {
            Self::Linear => 1,
            Self::Quadratic => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HybridOptions2d {
    pub physics: Physics,
    pub family: ElementFamily,
    pub partition: PartitionOptions2d,
    pub mesh: MeshOptions,
    /// A hybrid mesh is used only when at least this fraction of the area is structured (otherwise its transitions cost more
    /// than the regular blocks save); the plain unstructured mesh is returned and the reason reported.
    pub min_structured_fraction: f64,
    pub quality: QualityThresholds,
    /// Structured element edge relative to the size function (a Quad of edge `h` is about as accurate as a triangle of edge
    /// `0.87 h`, the mean the Delaunay mesher produces for the same size function).
    pub structured_scale: f64,
    /// Element edge multipliers along `x` and `y` in structured cells: `[6, 1]` for a slender member along `x` whose solution
    /// varies slowly along its axis (elongated quads there, isotropic triangles only around the features). `[1, 1]` is the
    /// safe default; an anisotropic choice is the caller's modelling decision (adaptive refinement corrects it).
    pub structured_aspect: [f64; 2],
    /// How often the structured divisions are doubled along an interface edge the unstructured mesher split further.
    pub max_refits: usize,
}

impl HybridOptions2d {
    pub fn new(physics: Physics, family: ElementFamily) -> Self {
        Self { physics, family, partition: PartitionOptions2d::default(), mesh: MeshOptions::default(), min_structured_fraction: 0.15, structured_scale: 1.0, structured_aspect: [1.0, 1.0], quality: QualityThresholds::default(), max_refits: 4 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Strategy {
    Hybrid,
    /// The plain unstructured mesh of the whole region, with why the hybrid one was not used.
    Unstructured { reason: String },
}

#[derive(Debug, Clone)]
pub struct HybridReport {
    pub strategy: Strategy,
    pub structured_cells: usize,
    pub unstructured_cells: usize,
    /// Structured share of the meshed area.
    pub structured_area_fraction: f64,
    pub structured_elements: usize,
    pub unstructured_elements: usize,
    pub transition_elements: usize,
    pub refits: usize,
    pub nodes: usize,
    pub quality: QualityReport,
    pub conformity: ConformityReport,
}

#[derive(Debug, Clone)]
pub struct HybridMesh2d {
    pub mesh: Mesh,
    pub labels: ElementLabels,
    pub report: HybridReport,
}

fn dist_to_curve(c: &Curve, p: [f64; 2]) -> f64 {
    match *c {
        Curve::Line { a, b } => {
            let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
            let l2 = ab[0] * ab[0] + ab[1] * ab[1];
            let t = if l2 > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
            (p[0] - a[0] - t * ab[0]).hypot(p[1] - a[1] - t * ab[1])
        }
        Curve::Arc { c: ctr, r, a0, a1 } => {
            let th = (p[1] - ctr[1]).atan2(p[0] - ctr[0]);
            let (lo, hi) = (a0.min(a1), a0.max(a1));
            // bring the angle into [lo, lo + 2 pi)
            let mut t = th;
            while t < lo {
                t += std::f64::consts::TAU;
            }
            while t >= lo + std::f64::consts::TAU {
                t -= std::f64::consts::TAU;
            }
            if t <= hi {
                ((p[0] - ctr[0]).hypot(p[1] - ctr[1]) - r).abs()
            } else {
                let (s, e) = (c.start(), c.end());
                (p[0] - s[0]).hypot(p[1] - s[1]).min((p[0] - e[0]).hypot(p[1] - e[1]))
            }
        }
    }
}

/// Node sets and surfaces by segment name, from the boundary faces of the meshed union: a boundary face belongs to the segment
/// both of whose corner nodes lie on it. Independent of which mesher produced the face.
pub fn assign_names(mesh: &mut Mesh, region: &Region) {
    let (lo, hi) = region.bounds();
    let tol = 1e-7 * (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    let segs: Vec<&Segment> = region.loops().flat_map(|l| l.segments.iter()).collect();
    mesh.node_sets.clear();
    mesh.surfaces.clear();
    for f in mesh.boundary_faces() {
        let (a, b) = (mesh.nodes[f.nodes[0]], mesh.nodes[f.nodes[1]]);
        let on = |s: &Segment, p: [f64; 3]| dist_to_curve(&s.curve, [p[0], p[1]]) <= tol;
        if let Some(s) = segs.iter().find(|s| on(s, a) && on(s, b)) {
            mesh.surfaces.entry(s.name.clone()).or_default().push(f.nodes.clone());
            mesh.node_sets.entry(s.name.clone()).or_default().extend(f.nodes.iter().copied());
        }
    }
    for set in mesh.node_sets.values_mut() {
        set.sort_unstable();
        set.dedup();
    }
}

fn axis_point(lo: f64, hi: f64, k: usize, n: usize) -> f64 {
    if k == 0 {
        lo
    } else if k == n {
        hi
    } else {
        lo + (hi - lo) * (k as f64 / n as f64)
    }
}

/// Divisions of every structured cell. A cell's `nx` is shared with the structured cells above and below it (they have one edge
/// in common, which both must divide the same way) and its `ny` with those to its left and right; a run of structured cells in
/// a column therefore shares `nx`, but an unstructured cell breaks the run, so far-apart blocks follow the size function
/// independently.
struct Divs {
    xroot: Vec<usize>,
    yroot: Vec<usize>,
    xcount: Vec<usize>,
    ycount: Vec<usize>,
    nxcells: usize,
}

impl Divs {
    fn cell(&self, ci: usize, cj: usize) -> usize {
        cj * self.nxcells + ci
    }
    fn nx(&self, ci: usize, cj: usize) -> usize {
        self.xcount[self.xroot[self.cell(ci, cj)]]
    }
    fn ny(&self, ci: usize, cj: usize) -> usize {
        self.ycount[self.yroot[self.cell(ci, cj)]]
    }
    /// Double the horizontal divisions of the run containing `(ci, cj)`; false at the cap.
    fn bump_x(&mut self, ci: usize, cj: usize) -> bool {
        let r = self.xroot[self.cell(ci, cj)];
        if self.xcount[r] >= 4096 {
            return false;
        }
        self.xcount[r] *= 2;
        true
    }
    fn bump_y(&mut self, ci: usize, cj: usize) -> bool {
        let r = self.yroot[self.cell(ci, cj)];
        if self.ycount[r] >= 4096 {
            return false;
        }
        self.ycount[r] *= 2;
        true
    }
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

fn divisions(grid: &CellGrid, size: &dyn Fn([f64; 2]) -> f64, quad_scale: f64, aspect: [f64; 2]) -> Divs {
    let (nxc, nyc) = (grid.nx(), grid.ny());
    let n = nxc * nyc;
    let (mut xparent, mut yparent): (Vec<usize>, Vec<usize>) = ((0..n).collect(), (0..n).collect());
    let (mut xcount, mut ycount) = (vec![1usize; n], vec![1usize; n]);
    let structured = |ci: usize, cj: usize| grid.at(ci, cj) == CellState::Structured;
    for cj in 0..nyc {
        for ci in 0..nxc {
            if !structured(ci, cj) {
                continue;
            }
            let (a, b) = grid.cell_box(ci, cj);
            // the cells are bisected until the size is within a factor of 1.4 across them: the geometric mean of the smallest
            // and the central size keeps a cell near its local target without over-resolving its coarse side
            let mut h = f64::INFINITY;
            for fx in [0.0, 0.5, 1.0] {
                for fy in [0.0, 0.5, 1.0] {
                    h = h.min(size([a[0] + fx * (b[0] - a[0]), a[1] + fy * (b[1] - a[1])]));
                }
            }
            let centre = size([0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]);
            let h = (h * centre).sqrt() * quad_scale;
            let c = cj * nxc + ci;
            xcount[c] = ((b[0] - a[0]) / (h * aspect[0])).round().max(1.0) as usize;
            ycount[c] = ((b[1] - a[1]) / (h * aspect[1])).round().max(1.0) as usize;
            if cj + 1 < nyc && structured(ci, cj + 1) {
                let (r1, r2) = (find(&mut xparent, c), find(&mut xparent, c + nxc));
                xparent[r2] = r1;
            }
            if ci + 1 < nxc && structured(ci + 1, cj) {
                let (r1, r2) = (find(&mut yparent, c), find(&mut yparent, c + 1));
                yparent[r2] = r1;
            }
        }
    }
    let xroot: Vec<usize> = (0..n).map(|c| find(&mut xparent, c)).collect();
    let yroot: Vec<usize> = (0..n).map(|c| find(&mut yparent, c)).collect();
    let (mut xmax, mut ymax) = (vec![1usize; n], vec![1usize; n]);
    for c in 0..n {
        xmax[xroot[c]] = xmax[xroot[c]].max(xcount[c]);
        ymax[yroot[c]] = ymax[yroot[c]].max(ycount[c]);
    }
    Divs { xroot, yroot, xcount: xmax, ycount: ymax, nxcells: nxc }
}

type Lat = (usize, usize);

struct DirEdge {
    from: Lat,
    to: Lat,
    interface: bool,
    parts: usize,
}

/// The unstructured regions: loops traced around the union of the unstructured cells, with the structured side of every
/// interface edge pre-split into its divisions.
fn trace_regions(region: &Region, grid: &CellGrid, divs: &Divs) -> Result<Vec<Region>, MeshError> {
    let pt = |l: Lat| [grid.xs[l.0], grid.ys[l.1]];
    let mut edges: Vec<DirEdge> = Vec::new();
    for cj in 0..grid.ny() {
        for ci in 0..grid.nx() {
            if grid.at(ci, cj) != CellState::Unstructured {
                continue;
            }
            let (i, j) = (ci as i64, cj as i64);
            // counter-clockwise around the cell: bottom, right, top, left, each with the neighbour across it
            let sides = [((ci, cj), (ci + 1, cj), (i, j - 1), true), ((ci + 1, cj), (ci + 1, cj + 1), (i + 1, j), false), ((ci + 1, cj + 1), (ci, cj + 1), (i, j + 1), true), ((ci, cj + 1), (ci, cj), (i - 1, j), false)];
            for (from, to, nb, horizontal) in sides {
                match grid.at_signed(nb.0, nb.1) {
                    CellState::Unstructured => {}
                    CellState::Structured => {
                        let (ni, nj) = (nb.0 as usize, nb.1 as usize);
                        edges.push(DirEdge { from, to, interface: true, parts: if horizontal { divs.nx(ni, nj) } else { divs.ny(ni, nj) } })
                    }
                    CellState::Void => edges.push(DirEdge { from, to, interface: false, parts: 1 }),
                }
            }
        }
    }
    if edges.is_empty() {
        return Err(MeshError::Failed("no unstructured cells".into()));
    }
    let mut out_of: HashMap<Lat, Vec<usize>> = HashMap::new();
    for (k, e) in edges.iter().enumerate() {
        out_of.entry(e.from).or_default().push(k);
    }
    if out_of.values().any(|v| v.len() > 1) {
        return Err(MeshError::Unsupported("two unstructured regions touch at a single grid vertex".into()));
    }
    let outer_segment_name = |a: [f64; 2], b: [f64; 2]| -> String {
        let mid = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])];
        region.outer.segments.iter().find(|s| dist_to_curve(&s.curve, mid) <= 1e-9 * (grid.xs.last().unwrap() - grid.xs[0]).max(1e-300)).map_or_else(|| "boundary".to_string(), |s| s.name.clone())
    };
    let mut used = vec![false; edges.len()];
    let mut loops: Vec<(Loop, f64)> = Vec::new();
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        let mut segs: Vec<Segment> = Vec::new();
        let mut area2 = 0.0;
        let mut k = start;
        loop {
            used[k] = true;
            let e = &edges[k];
            let (a, b) = (pt(e.from), pt(e.to));
            area2 += a[0] * b[1] - a[1] * b[0];
            if e.interface {
                let horizontal = e.from.1 == e.to.1;
                for q in 0..e.parts {
                    let p = |t: usize| -> [f64; 2] {
                        if horizontal {
                            [axis_point(a[0], b[0], t, e.parts), a[1]]
                        } else {
                            [a[0], axis_point(a[1], b[1], t, e.parts)]
                        }
                    };
                    segs.push(Segment { curve: Curve::line(p(q), p(q + 1)), name: "_interface".into() });
                }
            } else {
                segs.push(Segment { curve: Curve::line(a, b), name: outer_segment_name(a, b) });
            }
            let next = out_of.get(&e.to).and_then(|v| v.first().copied());
            match next {
                Some(n) if n == start => break,
                Some(n) if !used[n] => k = n,
                _ => return Err(MeshError::Failed("the boundary of the unstructured cells does not close".into())),
            }
        }
        loops.push((Loop::new(segs).map_err(MeshError::Failed)?, area2 * 0.5));
    }
    // outer loops (counter-clockwise, positive area) and the inner loops they enclose
    let mut regions: Vec<(Loop, f64, Vec<Loop>)> = loops.iter().filter(|l| l.1 > 0.0).map(|l| (l.0.clone(), l.1, Vec::new())).collect();
    for (inner, _) in loops.iter().filter(|l| l.1 < 0.0) {
        let p = inner.segments[0].curve.point(0.5);
        let owner = regions.iter_mut().filter(|r| r.0.contains(p)).min_by(|a, b| a.1.total_cmp(&b.1));
        match owner {
            Some(r) => r.2.push(inner.clone()),
            None => return Err(MeshError::Failed("an inner boundary has no enclosing region".into())),
        }
    }
    for h in &region.holes {
        let p = h.segments[0].curve.start();
        let owner = regions.iter_mut().filter(|r| r.0.contains(p)).min_by(|a, b| a.1.total_cmp(&b.1));
        match owner {
            Some(r) => r.2.push(h.clone()),
            None => return Err(MeshError::Failed("a hole lies outside every unstructured region".into())),
        }
    }
    regions.into_iter().map(|(o, _, holes)| Region::new(o, holes, region.material).map_err(MeshError::Failed)).collect()
}

/// The structured cells as one Quad block (nodes duplicated between cells; the weld joins them).
fn structured_part(region: &Region, grid: &CellGrid, divs: &Divs, physics: Physics, family: ElementFamily) -> Result<Option<Mesh>, MeshError> {
    let quad = family.quad();
    let p = family.order();
    let mut mesh = Mesh::new(physics);
    let mut conn: Vec<usize> = Vec::new();
    for cj in 0..grid.ny() {
        for ci in 0..grid.nx() {
            if grid.at(ci, cj) != CellState::Structured {
                continue;
            }
            let (nx, ny) = (divs.nx(ci, cj), divs.ny(ci, cj));
            let (x0, x1, y0, y1) = (grid.xs[ci], grid.xs[ci + 1], grid.ys[cj], grid.ys[cj + 1]);
            let mut ids: HashMap<(usize, usize), usize> = HashMap::new();
            let mut node = |mesh: &mut Mesh, a: usize, b: usize| -> usize {
                *ids.entry((a, b)).or_insert_with(|| mesh.add_node([axis_point(x0, x1, a, p * nx), axis_point(y0, y1, b, p * ny), 0.0]))
            };
            for j in 0..ny {
                for i in 0..nx {
                    let (a, b) = (p * i, p * j);
                    let c = [(a, b), (a + p, b), (a + p, b + p), (a, b + p)];
                    for &(u, v) in &c {
                        conn.push(node(&mut mesh, u, v));
                    }
                    if p == 2 {
                        for &(u, v) in &[(a + 1, b), (a + 2, b + 1), (a + 1, b + 2), (a, b + 1)] {
                            conn.push(node(&mut mesh, u, v));
                        }
                    }
                }
            }
        }
    }
    if conn.is_empty() {
        return Ok(None);
    }
    mesh.add_block(quad, conn, region.material, "structured").map_err(MeshError::Failed)?;
    Ok(Some(mesh))
}

fn block_origin_labels(mesh: &Mesh, structured_block: usize) -> ElementLabels {
    ElementLabels(mesh.blocks.iter().enumerate().flat_map(|(b, blk)| std::iter::repeat_n(if b == structured_block { RegionKind::Structured } else { RegionKind::Unstructured }, blk.n_elems())).collect())
}

/// Mark the elements that touch a node shared by the structured and an unstructured block as transition elements.
fn mark_transition(mesh: &Mesh, labels: &mut ElementLabels, structured_block: usize) {
    let mut in_structured = vec![false; mesh.nodes.len()];
    let mut in_other = vec![false; mesh.nodes.len()];
    for (b, blk) in mesh.blocks.iter().enumerate() {
        for &n in &blk.conn {
            if b == structured_block {
                in_structured[n] = true;
            } else {
                in_other[n] = true;
            }
        }
    }
    let mut g = 0;
    for blk in &mesh.blocks {
        for e in 0..blk.n_elems() {
            if blk.elem(e).iter().any(|&n| in_structured[n] && in_other[n]) {
                labels.0[g] = RegionKind::Transition;
            }
            g += 1;
        }
    }
}

fn mesh_area(mesh: &Mesh) -> f64 {
    let mut a = 0.0;
    for blk in &mesh.blocks {
        let nc = blk.kind.n_corners();
        for e in 0..blk.n_elems() {
            let c = &blk.elem(e)[..nc];
            let mut s = 0.0;
            for k in 0..nc {
                let (p, q) = (mesh.nodes[c[k]], mesh.nodes[c[(k + 1) % nc]]);
                s += p[0] * q[1] - p[1] * q[0];
            }
            a += 0.5 * s.abs();
        }
    }
    a
}

fn report_of(mesh: &Mesh, labels: &ElementLabels, strategy: Strategy, grid: Option<&CellGrid>, refits: usize) -> HybridReport {
    let adj = Adjacency::new(mesh);
    let quality = mesh.quality().with_progression(&adj);
    let conformity = check_conformity(mesh, &adj);
    let (sc, uc, frac) = grid.map_or((0, 0, 0.0), |g| {
        let (s, u) = (g.structured_area(), g.unstructured_area());
        (g.count(CellState::Structured), g.count(CellState::Unstructured), if s + u > 0.0 { s / (s + u) } else { 0.0 })
    });
    let hybrid = matches!(strategy, Strategy::Hybrid);
    HybridReport {
        strategy,
        structured_cells: sc,
        unstructured_cells: uc,
        structured_area_fraction: if hybrid { frac } else { 0.0 },
        structured_elements: labels.count(RegionKind::Structured),
        unstructured_elements: labels.count(RegionKind::Unstructured),
        transition_elements: labels.count(RegionKind::Transition),
        refits,
        nodes: mesh.nodes.len(),
        quality,
        conformity,
    }
}

/// Plain unstructured mesh of the whole region (Tri3 / Tri6), the fallback and the baseline of the hybrid one.
pub fn mesh_unstructured_2d(region: &Region, size: &dyn Fn([f64; 2]) -> f64, opt: &HybridOptions2d, reason: &str) -> Result<HybridMesh2d, MeshError> {
    let mesh = mesh_region(region, opt.physics, opt.family.tri(), size, opt.mesh).map_err(MeshError::Failed)?;
    let labels = ElementLabels::uniform(&mesh, RegionKind::Unstructured);
    let report = report_of(&mesh, &labels, Strategy::Unstructured { reason: reason.to_string() }, None, 0);
    report.quality.require(&opt.quality)?;
    Ok(HybridMesh2d { mesh, labels, report })
}

/// Hybrid structured/unstructured mesh of `region` (see the module docs). Falls back to the unstructured mesh, with the reason
/// in the report, when the region is not suitable or the construction cannot be verified.
pub fn mesh_hybrid_2d(region: &Region, size: &dyn Fn([f64; 2]) -> f64, opt: &HybridOptions2d) -> Result<HybridMesh2d, MeshError> {
    let fallback = |why: String| mesh_unstructured_2d(region, size, opt, &why);
    let grid = match classify_2d(region, size, &opt.partition) {
        Ok(g) => g,
        Err(MeshError::Unsupported(why)) => return fallback(why),
        Err(e) => return Err(e),
    };
    let (s_area, u_area) = (grid.structured_area(), grid.unstructured_area());
    if grid.count(CellState::Structured) == 0 || s_area / (s_area + u_area) < opt.min_structured_fraction {
        return fallback(format!("only {:.0} % of the area is regular (need {:.0} %)", 100.0 * s_area / (s_area + u_area).max(1e-300), 100.0 * opt.min_structured_fraction));
    }
    if grid.count(CellState::Unstructured) == 0 {
        // wholly regular: a structured mesh needs no unstructured part
        let divs = divisions(&grid, size, opt.structured_scale, opt.structured_aspect);
        let Some(part) = structured_part(region, &grid, &divs, opt.physics, opt.family)? else { return fallback("no structured cells".into()) };
        let (lo, hi) = region.bounds();
        let mut mesh = merge(&[part], 1e-9 * (hi[0] - lo[0]).hypot(hi[1] - lo[1]))?;
        assign_names(&mut mesh, region);
        let labels = ElementLabels::uniform(&mesh, RegionKind::Structured);
        let report = report_of(&mesh, &labels, Strategy::Hybrid, Some(&grid), 0);
        report.conformity.require_conforming()?;
        report.quality.require(&opt.quality)?;
        return Ok(HybridMesh2d { mesh, labels, report });
    }
    let (lo, hi) = region.bounds();
    let diag = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    let mut divs = divisions(&grid, size, opt.structured_scale, opt.structured_aspect);
    let mut last_problem = String::new();
    for refit in 0..=opt.max_refits {
        let attempt = (|| -> Result<HybridMesh2d, MeshError> {
            let regions = trace_regions(region, &grid, &divs)?;
            let mut parts: Vec<Mesh> = Vec::new();
            for r in &regions {
                parts.push(mesh_region(r, opt.physics, opt.family.tri(), size, opt.mesh).map_err(MeshError::Failed)?);
            }
            let n_unstructured_blocks = parts.len();
            let s = structured_part(region, &grid, &divs, opt.physics, opt.family)?.ok_or_else(|| MeshError::Failed("no structured cells".into()))?;
            parts.push(s);
            let mut mesh = merge(&parts, 1e-9 * diag)?;
            // blocks: each unstructured part contributes one block, then the structured one
            let structured_block = n_unstructured_blocks;
            assign_names(&mut mesh, region);
            let mut labels = block_origin_labels(&mesh, structured_block);
            mark_transition(&mesh, &mut labels, structured_block);
            let report = report_of(&mesh, &labels, Strategy::Hybrid, Some(&grid), refit);
            let area = mesh_area(&mesh);
            if (area - region.area()).abs() > 2e-3 * region.area() {
                return Err(MeshError::Failed(format!("the meshed area {area} differs from the region's {}", region.area())));
            }
            Ok(HybridMesh2d { mesh, labels, report })
        })();
        match attempt {
            Ok(h) if h.report.conformity.is_conforming() => {
                if let Err(e) = h.report.quality.require(&opt.quality) {
                    return fallback(format!("hybrid mesh rejected: {e}"));
                }
                return Ok(h);
            }
            Ok(h) => {
                // the unstructured mesher split an interface edge: double the structured divisions along it
                last_problem = h.report.conformity.summary();
                let tol = 1e-7 * diag;
                // each structured run whose interface edge was split further is doubled once per refit
                let mut bump_x: std::collections::BTreeSet<(usize, usize)> = Default::default();
                let mut bump_y: std::collections::BTreeSet<(usize, usize)> = Default::default();
                let structured = |ci: i64, cj: i64| grid.at_signed(ci, cj) == CellState::Structured;
                for &n in &h.report.conformity.hanging_nodes {
                    let p = h.mesh.nodes[n];
                    let inside = |lo: f64, hi: f64, v: f64| v > lo - tol && v < hi + tol;
                    for (i, &x) in grid.xs.iter().enumerate() {
                        if (x - p[0]).abs() <= tol {
                            // a vertical interface: the structured cell on either side, in the row holding p
                            for j in (0..grid.ny()).filter(|&j| inside(grid.ys[j], grid.ys[j + 1], p[1])) {
                                for ci in [i as i64 - 1, i as i64] {
                                    if structured(ci, j as i64) {
                                        bump_y.insert((ci as usize, j));
                                    }
                                }
                            }
                        }
                    }
                    for (j, &y) in grid.ys.iter().enumerate() {
                        if (y - p[1]).abs() <= tol {
                            for i in (0..grid.nx()).filter(|&i| inside(grid.xs[i], grid.xs[i + 1], p[0])) {
                                for cj in [j as i64 - 1, j as i64] {
                                    if structured(i as i64, cj) {
                                        bump_x.insert((i, cj as usize));
                                    }
                                }
                            }
                        }
                    }
                }
                let mut bumped = false;
                let mut seen_x = std::collections::BTreeSet::new();
                let mut seen_y = std::collections::BTreeSet::new();
                for &(ci, cj) in &bump_x {
                    if seen_x.insert(divs.xroot[divs.cell(ci, cj)]) {
                        bumped |= divs.bump_x(ci, cj);
                    }
                }
                for &(ci, cj) in &bump_y {
                    if seen_y.insert(divs.yroot[divs.cell(ci, cj)]) {
                        bumped |= divs.bump_y(ci, cj);
                    }
                }
                if !bumped {
                    break;
                }
            }
            Err(MeshError::Unsupported(why)) => return fallback(why),
            Err(e) => {
                last_problem = e.to_string();
                break;
            }
        }
    }
    fallback(format!("the hybrid construction could not be verified ({last_problem})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Elastic;

    pub(crate) fn steel() -> Elastic {
        Elastic::new(30e6, 0.3)
    }

    fn plate_with_hole() -> Region {
        Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), vec![Loop::circle([6.0, 2.0], 0.6, "hole").unwrap()], steel()).unwrap()
    }

    #[test]
    fn a_plate_with_a_hole_meshes_as_a_conforming_hybrid() {
        for family in [ElementFamily::Linear, ElementFamily::Quadratic] {
            let r = plate_with_hole();
            let opt = HybridOptions2d::new(Physics::PlaneStress { thickness: 0.1 }, family);
            let h = mesh_hybrid_2d(&r, &|_| 0.4, &opt).unwrap();
            assert_eq!(h.report.strategy, Strategy::Hybrid, "{family:?}");
            let c = &h.report.conformity;
            assert!(c.is_conforming() && c.components == 1 && c.unused_nodes == 0, "{family:?}: {}", c.summary());
            assert!(h.report.structured_elements > 0 && h.report.unstructured_elements > 0 && h.report.transition_elements > 0);
            assert!(h.report.quality.inverted == 0 && h.report.quality.min_scaled_jacobian > 0.2, "{}", h.report.quality.min_scaled_jacobian);
            // Quad and Tri blocks coexist
            let kinds: Vec<ElementKind> = h.mesh.blocks.iter().map(|b| b.kind).collect();
            assert!(kinds.contains(&family.quad()) && kinds.contains(&family.tri()));
            // every named boundary is present, outer edges and the hole
            for name in ["bottom", "right", "top", "left", "hole"] {
                assert!(h.mesh.surfaces.get(name).is_some_and(|v| !v.is_empty()), "missing {name}");
            }
            // area is the region's (the hole is an arc: quadratic elements follow it, linear ones cut chords)
            let tol = if family == ElementFamily::Linear { 5e-3 } else { 1e-3 };
            assert!((mesh_area(&h.mesh) - r.area()).abs() < tol * r.area());
        }
    }

    #[test]
    fn labels_cover_every_element_and_the_transition_separates_the_families() {
        let r = plate_with_hole();
        let h = mesh_hybrid_2d(&r, &|_| 0.4, &HybridOptions2d::new(Physics::PlaneStress { thickness: 0.1 }, ElementFamily::Quadratic)).unwrap();
        assert_eq!(h.labels.0.len(), h.mesh.n_elems());
        let rep = &h.report;
        assert_eq!(rep.structured_elements + rep.unstructured_elements + rep.transition_elements, h.mesh.n_elems());
        // no purely structured element shares a node with a purely unstructured one except through transition elements
        let adj = h.mesh.adjacency();
        for e in 0..h.mesh.n_elems() {
            if h.labels.0[e] == RegionKind::Structured {
                for &m in adj.neighbours(e) {
                    assert_ne!(h.labels.0[m as usize], RegionKind::Unstructured);
                }
            }
        }
    }

    #[test]
    fn a_curved_outer_boundary_falls_back_with_the_reason() {
        let r = Region::new(Loop::circle([0.0, 0.0], 1.0, "outer").unwrap(), vec![], steel()).unwrap();
        let h = mesh_hybrid_2d(&r, &|_| 0.2, &HybridOptions2d::new(Physics::PlaneStress { thickness: 1.0 }, ElementFamily::Quadratic)).unwrap();
        match &h.report.strategy {
            Strategy::Unstructured { reason } => assert!(reason.contains("rectilinear"), "{reason}"),
            s => panic!("{s:?}"),
        }
        assert!(h.report.conformity.is_conforming());
        assert_eq!(h.report.structured_elements, 0);
    }

    #[test]
    fn a_wholly_regular_region_is_a_pure_structured_mesh() {
        let r = Region::new(Loop::rectangle(0.0, 0.0, 4.0, 2.0).unwrap(), vec![], steel()).unwrap();
        let h = mesh_hybrid_2d(&r, &|_| 0.5, &HybridOptions2d::new(Physics::PlaneStress { thickness: 1.0 }, ElementFamily::Linear)).unwrap();
        assert_eq!(h.report.strategy, Strategy::Hybrid);
        assert_eq!(h.report.unstructured_elements + h.report.transition_elements, 0);
        assert!(h.mesh.blocks.iter().all(|b| b.kind == ElementKind::Quad4));
        assert!((mesh_area(&h.mesh) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn little_regular_area_means_the_unstructured_mesh() {
        // a hole that fills most of a small plate leaves no worthwhile block
        let r = Region::new(Loop::rectangle(0.0, 0.0, 3.0, 3.0).unwrap(), vec![Loop::circle([1.5, 1.5], 1.0, "hole").unwrap()], steel()).unwrap();
        let h = mesh_hybrid_2d(&r, &|_| 0.3, &HybridOptions2d::new(Physics::PlaneStress { thickness: 1.0 }, ElementFamily::Quadratic)).unwrap();
        assert!(matches!(h.report.strategy, Strategy::Unstructured { .. }));
    }

    #[test]
    fn an_l_shaped_plate_with_a_hole_meshes_hybrid() {
        let outer = Loop::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 3.0], [3.0, 3.0], [3.0, 10.0], [0.0, 10.0]], "edge").unwrap();
        let r = Region::new(outer, vec![Loop::circle([1.5, 1.5], 0.5, "hole").unwrap()], steel()).unwrap();
        let h = mesh_hybrid_2d(&r, &|_| 0.5, &HybridOptions2d::new(Physics::PlaneStress { thickness: 1.0 }, ElementFamily::Quadratic)).unwrap();
        assert_eq!(h.report.strategy, Strategy::Hybrid);
        assert!(h.report.conformity.is_conforming(), "{}", h.report.conformity.summary());
        assert!(h.report.structured_elements > 0);
        assert!((mesh_area(&h.mesh) - r.area()).abs() < 1e-3 * r.area());
    }
}
