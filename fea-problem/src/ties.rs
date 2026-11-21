//! Bonded ties between the bodies of an assembly problem (epic `docs/epic-assembly-feature-advice.md`, Phase C, first slice).
//!
//! A tie joins two flat faces of different bodies so they move as one, the cheap model of a bolted or welded joint. It is stored by
//! what the faces are (body, centroid, normal, area; [`FaceSig`]), never by a kernel or STEP face id, and resolved to the
//! cache's `faceN` when used, the way `solid::remap_faces` matches faces across an edit. At the solve the smaller face is the
//! slave: each of its surface nodes is constrained to the displacement interpolated over the master face's element face at the
//! node's projection (`fea_core::mpc`), so unequal meshes of the two bodies join without sharing nodes, and a tie on a face that
//! only partly overlaps its partner joins the overlap only.
use crate::assembly_advice::AssemblyAdvice;
use crate::assembly_mesh::{assembly_mesh, DEFAULT_CHORD};
use crate::problem::{Geometry, Problem};
use crate::solid::{add_triangle, face_stats, finish_stat, FaceStat, SolidCache};
use fea_core::mpc::Constraints;
use fea_core::{Dirichlet, Mesh};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

/// A flat face named by what it is: the body it belongs to (0-based, the product-structure order) and its area-weighted centroid,
/// outward normal and area, in the problem's units (converted with them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceSig {
    pub body: usize,
    pub centroid: [f64; 3],
    pub normal: [f64; 3],
    pub area: f64,
}

/// Two faces of different bodies joined. `gap` (signed, as the advice measured it) widens the distance within which the slave
/// face's nodes are taken to touch the master (bonded) or sets how far apart the bodies start (contact). `friction` is `None` for a
/// bonded joint; `Some(mu)` makes it a contact pair instead: the faces press and slide with Coulomb friction `mu` (`0` slides
/// freely) and can open, so a pull that separates the bodies carries no load across it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tie {
    pub a: FaceSig,
    pub b: FaceSig,
    #[serde(default)]
    pub gap: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friction: Option<f64>,
}

impl Tie {
    pub fn is_contact(&self) -> bool {
        self.friction.is_some()
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// The signature of a flat face from its triangles, `None` when it has no area or is not flat.
fn signature(body: usize, stat: &FaceStat) -> Option<FaceSig> {
    let n = len(stat.2);
    (stat.0 > 0.0 && n / stat.0 > 0.999).then(|| FaceSig { body, centroid: stat.1, normal: stat.2.map(|v| v / n), area: stat.0 })
}

/// The face of the cache that `sig` names: a flat face of that body with the same normal, in the same plane (within a
/// thousandth of the assembly's diagonal), whose centroid lies within a tenth of the face's size and whose area is within a
/// quarter; the nearest centroid wins.
fn find(c: &SolidCache, stats: &[FaceStat], sig: &FaceSig, diag: f64) -> Result<usize, String> {
    let range = c.body_faces().get(sig.body).cloned().ok_or_else(|| format!("body {} is not in the assembly", sig.body + 1))?;
    let size = sig.area.sqrt();
    let mut best: Option<(f64, usize)> = None;
    for f in range.filter(|f| stats[*f].0 > 0.0) {
        let Some(have) = signature(sig.body, &stats[f]) else { continue };
        let off = sub(have.centroid, sig.centroid);
        let across = dot(off, sig.normal).abs();
        let along = (dot(off, off) - across * across).max(0.0).sqrt();
        let ratio = have.area / sig.area;
        if dot(have.normal, sig.normal) > 0.999 && across <= 1e-3 * diag && along <= 0.1 * size && (0.8..=1.25).contains(&ratio) && best.is_none_or(|(d, _)| along < d) {
            best = Some((along, f));
        }
    }
    best.map(|(_, f)| f).ok_or_else(|| format!("no flat face of body {} matches the tied face (centroid {:.4?}, area {:.4e})", sig.body + 1, sig.centroid, sig.area))
}

fn diagonal(c: &SolidCache) -> f64 {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for v in &c.vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    len(sub(hi, lo))
}

/// The cache's face indices of a tie's two faces, the slave (smaller) first.
fn resolve(c: &SolidCache, stats: &[FaceStat], diag: f64, t: &Tie) -> Result<(usize, usize), String> {
    let (a, b) = (find(c, stats, &t.a, diag)?, find(c, stats, &t.b, diag)?);
    Ok(if stats[a].0 <= stats[b].0 { (a, b) } else { (b, a) })
}

/// The mesh surface name of face index `f`.
fn face_name(f: usize) -> String {
    format!("face{}", f + 1)
}

impl Problem {
    /// Check that the ties describe real, different bodies and resolve to faces of the stored assembly.
    pub(crate) fn validate_ties(&self) -> Result<(), String> {
        if self.ties.is_empty() {
            return Ok(());
        }
        if !self.contacts.is_empty() || !self.bushings.is_empty() {
            return Err("ties and 2D contact pairs or bushings do not combine yet: ties are for a plain static study of an assembly".into());
        }
        if self.ties.iter().any(Tie::is_contact) && self.ties.iter().any(|t| !t.is_contact()) {
            return Err("bonded ties and contact ties do not combine yet: make every joint bonded, or every joint a contact".into());
        }
        if self.any_plastic() || self.study.is_thermal() {
            return Err("ties are for a linear-elastic structural study: not an elastic-plastic or thermal one".into());
        }
        let Geometry::Solid { cache: Some(c), .. } = &self.geometry else { return Err("ties join the bodies of a STEP assembly".into()) };
        if c.bodies.len() < 2 {
            return Err("ties join the bodies of a STEP assembly, and this solid has one body".into());
        }
        let (stats, diag) = (face_stats(c), diagonal(c));
        for (i, t) in self.ties.iter().enumerate() {
            let ok = [&t.a, &t.b].iter().all(|s| s.area.is_finite() && s.area > 0.0 && s.centroid.iter().chain(&s.normal).all(|v| v.is_finite())) && t.gap.is_finite();
            if !ok {
                return Err(format!("tie {} has a face with no finite position, normal and area", i + 1));
            }
            if t.friction.is_some_and(|mu| !(0.0..=2.0).contains(&mu)) {
                return Err(format!("tie {}: friction must be in 0..2", i + 1));
            }
            if t.a.body == t.b.body {
                return Err(format!("tie {} joins two faces of body {}: a tie joins different bodies", i + 1, t.a.body + 1));
            }
            resolve(c, &stats, diag, t).map_err(|e| format!("tie {}: {e}", i + 1))?;
        }
        Ok(())
    }

    /// Bond the two faces of a `Tie` advice item (read from the problem's own STEP text, so its face keys are the ones
    /// `assembly_advice` gave). The faces are stored by signature in the problem's units; a tie already present is not added twice.
    pub fn apply_tie(&mut self, advice: &AssemblyAdvice) -> Result<(), String> {
        self.add_joint(advice, None)
    }

    /// As [`apply_tie`](Self::apply_tie), but the two faces become a contact pair with Coulomb friction `friction` (they press and
    /// slide and can open) instead of a bond.
    pub fn apply_contact(&mut self, advice: &AssemblyAdvice, friction: f64) -> Result<(), String> {
        self.add_joint(advice, Some(friction))
    }

    fn add_joint(&mut self, advice: &AssemblyAdvice, friction: Option<f64>) -> Result<(), String> {
        let AssemblyAdvice::Tie { a, b, gap, .. } = advice else { return Err("only a Tie advice item can be applied as a tie or a contact".into()) };
        let Geometry::Solid { step, cache: Some(c) } = &self.geometry else { return Err("ties join the bodies of a STEP assembly".into()) };
        if !c.is_current(step) {
            return Err("the STEP surface cache is out of date: open the STEP file again".into());
        }
        let mesh = assembly_mesh(step, DEFAULT_CHORD)?;
        if mesh.bodies() != c.bodies.len() {
            return Err(format!("the STEP text has {} bodies but the stored solid has {}", mesh.bodies(), c.bodies.len()));
        }
        let scale = c.coordinate_scale;
        let sig = |key: &solid_kernel::features::FaceKey| -> Result<FaceSig, String> {
            let mut stat: FaceStat = (0.0, [0.0; 3], [0.0; 3]);
            for t in mesh.triangles_of(&[*key]) {
                add_triangle(&mut stat, mesh.triangles[t].map(|i| mesh.vertices[i]));
            }
            finish_stat(&mut stat);
            let s = signature(key.body, &stat).ok_or_else(|| format!("body {} face {} is not a flat face", key.body + 1, key.face))?;
            Ok(FaceSig { centroid: s.centroid.map(|v| v * scale), area: s.area * scale * scale, ..s })
        };
        let tie = Tie { a: sig(a)?, b: sig(b)?, gap: gap * scale, friction };
        let (stats, diag) = (face_stats(c), diagonal(c));
        resolve(c, &stats, diag, &tie).map_err(|e| format!("the tied faces are not in the stored solid: {e}"))?;
        if !self.ties.contains(&tie) {
            self.ties.push(tie);
        }
        Ok(())
    }
}

/// Corner weights (`l1, l2, l3`) of `x` over triangle `abc` when its projection falls inside (a tolerance of 1e-6 outside is
/// taken as on the edge), with the distance of `x` from the triangle's plane.
fn barycentric(a: [f64; 3], b: [f64; 3], c: [f64; 3], x: [f64; 3]) -> Option<([f64; 3], f64)> {
    let (v0, v1, v2) = (sub(b, a), sub(c, a), sub(x, a));
    let (d00, d01, d11, d20, d21) = (dot(v0, v0), dot(v0, v1), dot(v1, v1), dot(v2, v0), dot(v2, v1));
    let denom = d00 * d11 - d01 * d01;
    if denom <= 0.0 {
        return None;
    }
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    let u = 1.0 - v - w;
    const EDGE: f64 = 1e-6;
    if u < -EDGE || v < -EDGE || w < -EDGE {
        return None;
    }
    let foot = [a[0] + v * v0[0] + w * v1[0], a[1] + v * v0[1] + w * v1[1], a[2] + v * v0[2] + w * v1[2]];
    Some(([u, v, w], len(sub(x, foot))))
}

/// Shape-function weights of a surface element over its nodes: three corners are linear, six (corners, then the midsides of
/// ab, bc, ca) are the quadratic triangle's.
fn weights(l: [f64; 3], nodes: usize) -> Vec<f64> {
    if nodes == 3 {
        return l.to_vec();
    }
    let [u, v, w] = l;
    vec![u * (2.0 * u - 1.0), v * (2.0 * v - 1.0), w * (2.0 * w - 1.0), 4.0 * u * v, 4.0 * v * w, 4.0 * w * u]
}

/// The constraint equations of the problem's ties on `mesh` (3 per slave node: its displacement is the master element's
/// interpolation), with a note for every dof a prescribed support kept from being tied. A tie that joins no node is an error: the
/// faces do not touch within the tolerance, or the mesh is too coarse to put a node on one of them.
pub(crate) fn constraints(p: &Problem, mesh: &Mesh, bc: &Dirichlet) -> Result<(Constraints, Vec<String>), String> {
    let Geometry::Solid { cache: Some(c), .. } = &p.geometry else { return Err("ties join the bodies of a STEP assembly".into()) };
    let (stats, diag) = (face_stats(c), diagonal(c));
    let mut out = Constraints::new();
    let mut slaved: HashSet<usize> = HashSet::new();
    let mut skipped = 0usize;
    for (i, t) in p.ties.iter().enumerate().filter(|(_, t)| !t.is_contact()) {
        let (slave, master) = resolve(c, &stats, diag, t).map_err(|e| format!("tie {}: {e}", i + 1))?;
        let faces = |f: usize| -> Result<&Vec<Vec<usize>>, String> {
            mesh.surfaces.get(&face_name(f)).filter(|l| !l.is_empty()).ok_or_else(|| format!("tie {}: {} has no boundary elements at this mesh size: reduce element size", i + 1, face_name(f)))
        };
        let (slave_faces, master_faces) = (faces(slave)?, faces(master)?);
        let tol = t.gap.abs() + 0.1 * p.mesh.size;
        let boxes: Vec<([f64; 3], [f64; 3])> = master_faces.iter().map(|f| {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for &n in &f[..3] {
                for k in 0..3 {
                    lo[k] = lo[k].min(mesh.nodes[n][k] - tol);
                    hi[k] = hi[k].max(mesh.nodes[n][k] + tol);
                }
            }
            (lo, hi)
        }).collect();
        let nodes: BTreeSet<usize> = slave_faces.iter().flatten().copied().collect();
        let mut joined = 0;
        for &s in &nodes {
            let x = mesh.nodes[s];
            let hit = master_faces.iter().zip(&boxes).filter(|(_, (lo, hi))| (0..3).all(|k| x[k] >= lo[k] && x[k] <= hi[k])).find_map(|(f, _)| {
                barycentric(mesh.nodes[f[0]], mesh.nodes[f[1]], mesh.nodes[f[2]], x).filter(|(_, d)| *d <= tol).map(|(l, _)| (f, l))
            });
            let Some((f, l)) = hit else { continue };
            let w = weights(l, f.len());
            let mut tied = false;
            for comp in 0..3 {
                let dof = s * 3 + comp;
                if bc.fixed[dof] {
                    skipped += 1;
                } else if slaved.insert(dof) {
                    out.add_equation(dof, f.iter().zip(&w).map(|(m, w)| (m * 3 + comp, *w)).collect(), 0.0);
                    tied = true;
                }
            }
            joined += usize::from(tied);
        }
        if joined == 0 && !nodes.is_empty() && !nodes.iter().all(|n| (0..3).all(|k| bc.fixed[n * 3 + k])) {
            return Err(format!("tie {}: {} and {} join no node: the faces do not touch within {tol:.3e} or lie beyond one another's edge", i + 1, face_name(slave), face_name(master)));
        }
    }
    let notes = if skipped > 0 { vec![format!("{skipped} tie dofs lie on a prescribed support and stay as supported; the support, not the tie, holds them.")] } else { Vec::new() };
    Ok((out, notes))
}

/// A contact tie's two mesh surfaces (`faceN`), its Coulomb friction and its gap, in the tie's order.
pub(crate) struct ContactJoint {
    pub a: String,
    pub b: String,
    pub friction: f64,
    pub gap: f64,
}

/// The contact ties of a problem resolved to the surfaces of the stored assembly.
pub(crate) fn contact_joints(p: &Problem) -> Result<Vec<ContactJoint>, String> {
    let Some(Geometry::Solid { cache: Some(c), .. }) = p.ties.iter().any(Tie::is_contact).then_some(&p.geometry) else { return Ok(Vec::new()) };
    let (stats, diag) = (face_stats(c), diagonal(c));
    p.ties
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.friction.map(|mu| (i, t, mu)))
        .map(|(i, t, friction)| {
            let (a, b) = resolve(c, &stats, diag, t).map_err(|e| format!("tie {}: {e}", i + 1))?;
            Ok(ContactJoint { a: face_name(a), b: face_name(b), friction, gap: t.gap })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solid::SolidCache;
    use fea_core::Physics;
    use solid_kernel::SurfaceMesh;

    /// Two unit-height boxes stacked in z, the upper one shifted 0.5 in x so its bottom face overlaps the lower one's top over
    /// half of it. Body 1's top is `face6`, body 2's bottom `face11`.
    fn shifted() -> Problem {
        let mut m = SurfaceMesh::default();
        let quads = |m: &mut SurfaceMesh, x0: f64, z: f64, up: bool, tag: usize| {
            let p = |x: f64, y: f64| [x0 + x, y, z];
            let (a, b, c, d) = (p(0., 0.), p(1., 0.), p(1., 1.), p(0., 1.));
            if up { m.push_triangle(a, b, c, tag); m.push_triangle(a, c, d, tag); } else { m.push_triangle(a, c, b, tag); m.push_triangle(a, d, c, tag); }
        };
        // Only the two tied faces matter to the constraints; the rest of each box is closed by faces the test never reads.
        for (body, (x0, z)) in [(0.0, 0.0), (0.5, 1.0)].into_iter().enumerate() {
            let first = body * 6;
            quads(&mut m, x0, z, false, first + 4);
            quads(&mut m, x0, z + 1.0, true, first + 5);
        }
        let mut cache = SolidCache::new("", m, 0.01);
        cache.bodies = vec![6, 6];
        let mut p = crate::templates::blank();
        p.geometry = Geometry::Solid { step: String::new(), cache: Some(cache) };
        p.mesh.size = 0.5;
        p.ties = vec![Tie {
            a: FaceSig { body: 0, centroid: [0.5, 0.5, 1.0], normal: [0.0, 0.0, 1.0], area: 1.0 },
            b: FaceSig { body: 1, centroid: [1.0, 0.5, 1.0], normal: [0.0, 0.0, -1.0], area: 1.0 },
            gap: 0.0,
            friction: None,
        }];
        p
    }

    /// A strip of triangles over `xs` x {0, 1} at height 1 (nodes `base..`, two per x, bottom then top).
    fn strip(mesh: &mut Mesh, name: &str, xs: &[f64]) -> usize {
        let base = mesh.nodes.len();
        for &x in xs {
            mesh.nodes.push([x, 0.0, 1.0]);
            mesh.nodes.push([x, 1.0, 1.0]);
        }
        let tris = (0..xs.len() - 1).flat_map(|i| {
            let n = base + 2 * i;
            [vec![n, n + 2, n + 3], vec![n, n + 3, n + 1]]
        }).collect();
        mesh.surfaces.insert(name.into(), tris);
        base
    }

    #[test]
    fn only_the_nodes_over_the_other_face_are_tied_and_the_weights_reproduce_their_position() {
        let p = shifted();
        let mut mesh = Mesh::new(Physics::Solid);
        let slave = strip(&mut mesh, "face6", &[0.0, 0.25, 0.5, 0.75, 1.0]);
        let master = strip(&mut mesh, "face11", &[0.5, 1.0, 1.5]);
        let bc = Dirichlet::new(mesh.nodes.len(), 3);
        let (cons, notes) = constraints(&p, &mesh, &bc).unwrap();
        assert!(notes.is_empty());
        // face6 and face11 have equal area: the first is the slave. x = 0.5, 0.75, 1.0 lie over the master, 0 and 0.25 do not.
        let tied: BTreeSet<usize> = cons.equations.iter().map(|e| e.slave / 3).collect();
        let want: BTreeSet<usize> = (2..5).flat_map(|i| [slave + 2 * i, slave + 2 * i + 1]).collect();
        assert_eq!(tied, want);
        assert_eq!(cons.equations.len(), 3 * want.len());
        for e in &cons.equations {
            let (s, comp) = (e.slave / 3, e.slave % 3);
            assert!(e.terms.iter().all(|(m, _)| m % 3 == comp && m / 3 >= master));
            assert!((e.terms.iter().map(|t| t.1).sum::<f64>() - 1.0).abs() < 1e-12);
            // the interpolation of the master's own positions gives the slave's position
            let at: Vec<f64> = (0..2).map(|k| e.terms.iter().map(|(m, w)| w * mesh.nodes[m / 3][k]).sum()).collect();
            assert!((at[0] - mesh.nodes[s][0]).abs() < 1e-12 && (at[1] - mesh.nodes[s][1]).abs() < 1e-12, "{at:?} vs {:?}", mesh.nodes[s]);
        }
    }

    #[test]
    fn a_support_on_a_slave_node_is_kept_and_reported() {
        let p = shifted();
        let mut mesh = Mesh::new(Physics::Solid);
        let slave = strip(&mut mesh, "face6", &[0.0, 0.25, 0.5, 0.75, 1.0]);
        strip(&mut mesh, "face11", &[0.5, 1.0, 1.5]);
        let mut bc = Dirichlet::new(mesh.nodes.len(), 3);
        let held = (slave + 4) * 3 + 2; // z of the slave node at x = 0.5, y = 0
        bc.fixed[held] = true;
        let (cons, notes) = constraints(&p, &mesh, &bc).unwrap();
        assert!(cons.equations.iter().all(|e| e.slave != held));
        assert_eq!(cons.equations.len(), 3 * 6 - 1);
        assert!(notes[0].starts_with("1 tie dofs"), "{notes:?}");
    }
}
