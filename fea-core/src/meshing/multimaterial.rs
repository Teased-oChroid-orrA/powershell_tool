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
use crate::geometry::{Curve, Region};
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
    if parts.is_empty() {
        return Err(MeshError::Failed("no material regions".into()));
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(p) = parts.iter().find(|p| !seen.insert(p.name.as_str())) {
        return Err(MeshError::Failed(format!("the region name '{}' is used twice", p.name)));
    }
    let assemble = |meshes: Vec<HybridMesh2d>, strategy: Strategy| -> Result<(MultiMaterialMesh2d, bool), MeshError> {
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
        let (lo, hi) = parts.iter().fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), p| {
            let (l, h) = p.region.bounds();
            ([lo[0].min(l[0]), lo[1].min(l[1])], [hi[0].max(h[0]), hi[1].max(h[1])])
        });
        let tol = 1e-9 * (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
        // positions of every part's nodes, to find the shared ones after the weld
        let part_nodes: Vec<Vec<[f64; 3]>> = pieces.iter().zip(&members).map(|(m, used)| used.iter().map(|&n| m.nodes[n]).collect()).collect();
        let mut mesh = merge(&pieces, tol)?;
        let conformity = mesh.conformity();
        let ok = conformity.is_conforming();
        let regions: Vec<&Region> = parts.iter().map(|p| &p.region).collect();
        assign_names_of(&mut mesh, &regions);
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
    // Cells touching a shared boundary are forced unstructured on both sides: the Delaunay mesher discretises a boundary from
    // the size function and the geometry alone, so both sides place the same interface nodes.
    let mut hybrid_opt = opt.clone();
    for (i, a) in parts.iter().enumerate() {
        for b in &parts[i + 1..] {
            for sa in a.region.loops().flat_map(|l| l.segments.iter()) {
                if b.region.loops().flat_map(|l| l.segments.iter()).any(|sb| same_curve(&sa.curve, &sb.curve)) {
                    let (p, q) = (sa.curve.start(), sa.curve.end());
                    hybrid_opt.partition.extra_boxes.push(([p[0].min(q[0]), p[1].min(q[1])], [p[0].max(q[0]), p[1].max(q[1])]));
                }
            }
        }
    }
    let hybrid: Result<Vec<HybridMesh2d>, MeshError> = parts.iter().map(|p| mesh_hybrid_2d(&p.region, size, &hybrid_opt)).collect();
    let why = match hybrid {
        Ok(meshes) => {
            // hybrid when any region kept structured blocks (the others fell back on their own, with their reasons in the labels)
            let any_hybrid = meshes.iter().any(|m| m.report.strategy == Strategy::Hybrid);
            let strategy = if any_hybrid { Strategy::Hybrid } else { Strategy::Unstructured { reason: "no region is suitable for structured blocks".into() } };
            match assemble(meshes, strategy) {
                Ok((m, true)) if m.quality.require(&opt.quality).is_ok() => return Ok(m),
                Ok((m, true)) => format!("quality gate failed on the welded union: {}", m.quality.require(&opt.quality).unwrap_err()),
                Ok((m, false)) => format!("the hybrid parts do not weld conformingly ({})", m.conformity.summary()),
                Err(e) => format!("welding the hybrid parts failed: {e}"),
            }
        }
        Err(e) => format!("a hybrid part failed: {e}"),
    };
    let meshes = parts.iter().map(|p| mesh_unstructured_2d(&p.region, size, opt, &why)).collect::<Result<Vec<_>, _>>()?;
    match assemble(meshes, Strategy::Unstructured { reason: why })? {
        (m, true) => {
            m.quality.require(&opt.quality)?;
            Ok(m)
        }
        (m, false) => Err(MeshError::NonConforming(format!("the material interfaces do not match: {}", m.conformity.summary()))),
    }
}
