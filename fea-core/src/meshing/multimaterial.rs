//! Meshes of a planar body made of several materials: one [`Region`] per material, adjacent regions sharing the geometry of
//! their common boundary. Each region is meshed on its own (hybrid, falling back to unstructured) and the parts are welded.
//! The two sides of a material interface are conforming only if both discretise the shared boundary identically; the
//! unstructured mesher does (its boundary nodes depend on the size function and the boundary geometry alone), a structured
//! block against a different interface discretisation may not. So the union is verified, and if the hybrid parts do not weld
//! conformingly every part is meshed unstructured instead and the reason is reported. A non-conforming union is never returned.

use super::adjacency::ConformityReport;
use super::hybrid::{assign_names_of, mesh_hybrid_2d, mesh_unstructured_2d, HybridMesh2d, HybridOptions2d, Strategy};
use super::quality::QualityReport;
use super::weld::merge;
use super::{ElementLabels, MeshError};
use crate::geometry::{Curve, Loop, Region, Segment};
use crate::mesh::Mesh;

/// The same boundary piece seen from the other region: equal endpoints in either order (lines), same circle and extent (arcs).
fn same_curve(a: &Curve, b: &Curve) -> bool {
    let close = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) <= 1e-9 * (1.0 + p[0].abs() + p[1].abs());
    match (a, b) {
        (Curve::Line { .. }, Curve::Line { .. }) => (close(a.start(), b.start()) && close(a.end(), b.end())) || (close(a.start(), b.end()) && close(a.end(), b.start())),
        (Curve::Arc { c: c1, r: r1, .. }, Curve::Arc { c: c2, r: r2, .. }) => {
            close(*c1, *c2) && (r1 - r2).abs() <= 1e-9 * r1 && ((close(a.start(), b.start()) && close(a.end(), b.end())) || (close(a.start(), b.end()) && close(a.end(), b.start())))
        }
        _ => false,
    }
}

/// One material region of the body. `name` prefixes the names of its blocks; the material is the region's own.
pub struct MaterialRegion2d {
    pub name: String,
    pub region: Region,
}

pub struct MultiMaterialMesh2d {
    pub mesh: Mesh,
    pub labels: ElementLabels,
    pub strategy: Strategy,
    pub quality: QualityReport,
    pub conformity: ConformityReport,
    /// Nodes shared by two material regions, by pair of region names.
    pub interfaces: Vec<(String, String, Vec<usize>)>,
}

/// Mesh `parts` with the size function `size` and weld them. Region names must be distinct.
pub fn mesh_multi_material_2d(parts: &[MaterialRegion2d], size: &dyn Fn([f64; 2]) -> f64, opt: &HybridOptions2d) -> Result<MultiMaterialMesh2d, MeshError> {
    mesh_parts(parts, size, opt, true)
}

/// [`mesh_multi_material_2d`] without structured blocks: every part is meshed by the unstructured mesher, which places the nodes
/// of a shared boundary from the size function and the geometry alone, so the interfaces weld conformingly.
pub fn mesh_multi_material_unstructured_2d(parts: &[MaterialRegion2d], size: &dyn Fn([f64; 2]) -> f64, opt: &HybridOptions2d) -> Result<MultiMaterialMesh2d, MeshError> {
    mesh_parts(parts, size, opt, false)
}

/// Parameter of `p` along `c` when it lies on it (within `tol`), `None` otherwise.
fn param_on(c: &Curve, p: [f64; 2], tol: f64) -> Option<f64> {
    match *c {
        Curve::Line { a, b } => {
            let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
            let t = (ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]);
            let q = c.point(t.clamp(0.0, 1.0));
            ((q[0] - p[0]).hypot(q[1] - p[1]) <= tol).then_some(t.clamp(0.0, 1.0))
        }
        Curve::Arc { c: ctr, r, a0, a1 } => {
            if ((p[0] - ctr[0]).hypot(p[1] - ctr[1]) - r).abs() > tol {
                return None;
            }
            let sweep = a1 - a0;
            let th = (p[1] - ctr[1]).atan2(p[0] - ctr[0]);
            let delta = if sweep >= 0.0 { th - a0 } else { a0 - th }.rem_euclid(std::f64::consts::TAU);
            if delta <= sweep.abs() {
                Some(delta / sweep.abs())
            } else if std::f64::consts::TAU - delta <= tol / r {
                Some(0.0) // just before the start
            } else {
                None
            }
        }
    }
}

/// Corner positions of a mesh (the vertices of its elements; mid-side nodes follow from them).
fn corner_points(mesh: &Mesh) -> Vec<[f64; 2]> {
    let mut seen = std::collections::BTreeSet::new();
    for blk in &mesh.blocks {
        let nc = blk.kind.n_corners();
        for e in 0..blk.n_elems() {
            seen.extend(blk.elem(e)[..nc].iter().copied());
        }
    }
    seen.into_iter().map(|n| [mesh.nodes[n][0], mesh.nodes[n][1]]).collect()
}

/// The regions with every boundary segment they share with another region cut at the corner nodes either side's mesh placed on
/// it. Both sides then own every one of those points as a vertex, so a mesh that put a node on one side only (the unstructured
/// mesher splits a segment where it is encroached, which depends on the interior) gets the other side to match. `None` when
/// nothing would change.
fn split_shared(regions: &[Region], meshes: &[Mesh], tol: f64) -> Result<Option<Vec<Region>>, MeshError> {
    let corners: Vec<Vec<[f64; 2]>> = meshes.iter().map(corner_points).collect();
    let mut changed = false;
    let mut out = Vec::with_capacity(regions.len());
    for (i, r) in regions.iter().enumerate() {
        let split_loop = |l: &Loop| -> Result<Loop, MeshError> {
            let mut segs = Vec::new();
            for s in &l.segments {
                let partners: Vec<usize> = (0..regions.len()).filter(|&j| j != i && regions[j].loops().flat_map(|l| l.segments.iter()).any(|sb| same_curve(&s.curve, &sb.curve))).collect();
                let mut ts: Vec<f64> = Vec::new();
                if !partners.is_empty() {
                    for j in std::iter::once(i).chain(partners.iter().copied()) {
                        ts.extend(corners[j].iter().filter_map(|&p| param_on(&s.curve, p, tol)));
                    }
                }
                ts.sort_by(f64::total_cmp);
                let len = s.curve.length();
                let mut cuts = vec![0.0];
                for t in ts {
                    if (t - cuts.last().copied().unwrap_or(0.0)) * len > 10.0 * tol && (1.0 - t) * len > 10.0 * tol {
                        cuts.push(t);
                    }
                }
                cuts.push(1.0);
                for w in cuts.windows(2) {
                    segs.push(Segment { curve: s.curve.sub(w[0], w[1]), name: s.name.clone() });
                }
            }
            Loop::new(segs).map_err(MeshError::Failed)
        };
        // a segment already cut at all of those points is not a change: compare the segment counts
        let outer = split_loop(&r.outer)?;
        let holes = r.holes.iter().map(&split_loop).collect::<Result<Vec<_>, _>>()?;
        let before: usize = r.loops().map(|l| l.segments.len()).sum();
        let after: usize = std::iter::once(&outer).chain(holes.iter()).map(|l| l.segments.len()).sum();
        changed |= after > before;
        out.push(Region::new(outer, holes, r.material).map_err(MeshError::Failed)?);
    }
    Ok(changed.then_some(out))
}

/// Rounds of cutting the shared boundaries and meshing again before the weld is declared non-conforming.
const MAX_ROUNDS: usize = 6;

fn mesh_parts(parts: &[MaterialRegion2d], size: &dyn Fn([f64; 2]) -> f64, opt: &HybridOptions2d, try_hybrid: bool) -> Result<MultiMaterialMesh2d, MeshError> {
    if parts.is_empty() {
        return Err(MeshError::Failed("no material regions".into()));
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(p) = parts.iter().find(|p| !seen.insert(p.name.as_str())) {
        return Err(MeshError::Failed(format!("the region name '{}' is used twice", p.name)));
    }
    let (lo, hi) = parts.iter().fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), p| {
        let (l, h) = p.region.bounds();
        ([lo[0].min(l[0]), lo[1].min(l[1])], [hi[0].max(h[0]), hi[1].max(h[1])])
    });
    let tol = 1e-9 * (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    // The welded union of `meshes` (one per region), conforming or not.
    let assemble = |regions: &[Region], meshes: Vec<HybridMesh2d>, strategy: Strategy| -> Result<(MultiMaterialMesh2d, bool), MeshError> {
        let mut labels = Vec::new();
        let mut pieces = Vec::new();
        let mut members: Vec<Vec<usize>> = Vec::new();
        for (part, mut h) in parts.iter().zip(meshes) {
            for blk in &mut h.mesh.blocks {
                blk.name = format!("{}/{}", part.name, blk.name);
            }
            let used: std::collections::BTreeSet<usize> = h.mesh.blocks.iter().flat_map(|b| b.conn.iter().copied()).collect();
            members.push(used.into_iter().collect());
            labels.extend(h.labels.0);
            pieces.push(h.mesh);
        }
        // positions of every part's nodes, to find the shared ones after the weld
        let part_nodes: Vec<Vec<[f64; 3]>> = pieces.iter().zip(&members).map(|(m, used)| used.iter().map(|&n| m.nodes[n]).collect()).collect();
        let mut mesh = merge(&pieces, tol)?;
        let conformity = mesh.conformity();
        let ok = conformity.is_conforming();
        let refs: Vec<&Region> = regions.iter().collect();
        assign_names_of(&mut mesh, &refs);
        let mut interfaces = Vec::new();
        for i in 0..parts.len() {
            for j in i + 1..parts.len() {
                let mut shared: Vec<usize> = Vec::new();
                for x in &part_nodes[i] {
                    if part_nodes[j].iter().any(|y| (0..3).all(|c| (x[c] - y[c]).abs() <= tol)) {
                        if let Some(n) = mesh.nodes.iter().position(|z| (0..3).all(|c| (x[c] - z[c]).abs() <= tol)) {
                            shared.push(n);
                        }
                    }
                }
                shared.sort_unstable();
                shared.dedup();
                if !shared.is_empty() {
                    interfaces.push((parts[i].name.clone(), parts[j].name.clone(), shared));
                }
            }
        }
        let quality = mesh.quality();
        Ok((MultiMaterialMesh2d { mesh, labels: ElementLabels(labels), strategy, quality, conformity, interfaces }, ok))
    };
    // One strategy to a conforming union: mesh every region, weld, and while the weld does not match cut the shared boundaries
    // at the nodes both sides placed and mesh again. `Err(why)` when it never matches or fails a gate.
    let attempt = |hybrid: bool, reason: &str| -> Result<MultiMaterialMesh2d, String> {
        let mut regions: Vec<Region> = parts.iter().map(|p| p.region.clone()).collect();
        for _ in 0..MAX_ROUNDS {
            let mut hybrid_opt = opt.clone();
            if hybrid {
                // Cells touching a shared boundary are forced unstructured on both sides.
                for (i, a) in regions.iter().enumerate() {
                    for b in &regions[i + 1..] {
                        for sa in a.loops().flat_map(|l| l.segments.iter()) {
                            if b.loops().flat_map(|l| l.segments.iter()).any(|sb| same_curve(&sa.curve, &sb.curve)) {
                                let (p, q) = (sa.curve.start(), sa.curve.end());
                                hybrid_opt.partition.extra_boxes.push(([p[0].min(q[0]), p[1].min(q[1])], [p[0].max(q[0]), p[1].max(q[1])]));
                            }
                        }
                    }
                }
            }
            let meshes: Result<Vec<HybridMesh2d>, MeshError> = regions
                .iter()
                .map(|r| if hybrid { mesh_hybrid_2d(r, size, &hybrid_opt) } else { mesh_unstructured_2d(r, size, opt, reason) })
                .collect();
            let meshes = meshes.map_err(|e| format!("a part failed: {e}"))?;
            let strategy = if hybrid && meshes.iter().any(|m| m.report.strategy == Strategy::Hybrid) {
                Strategy::Hybrid
            } else {
                Strategy::Unstructured { reason: if hybrid { "no region is suitable for structured blocks".into() } else { reason.to_string() } }
            };
            let plain: Vec<Mesh> = meshes.iter().map(|m| m.mesh.clone()).collect();
            let (m, ok) = assemble(&regions, meshes, strategy).map_err(|e| format!("welding the parts failed: {e}"))?;
            if ok {
                return m.quality.require(&opt.quality).map(|()| m).map_err(|e| format!("quality gate failed on the welded union: {e}"));
            }
            match split_shared(&regions, &plain, tol).map_err(|e| e.to_string())? {
                Some(next) => regions = next,
                None => return Err(format!("the parts do not weld conformingly ({})", m.conformity.summary())),
            }
        }
        Err("the parts did not weld conformingly after repeated cuts of the shared boundaries".into())
    };
    let why = if try_hybrid {
        match attempt(true, "") {
            Ok(m) => return Ok(m),
            Err(why) => why,
        }
    } else {
        "unstructured meshing was requested".to_string()
    };
    attempt(false, &why).map_err(|e| MeshError::NonConforming(format!("the material interfaces do not match: {e}")))
}
