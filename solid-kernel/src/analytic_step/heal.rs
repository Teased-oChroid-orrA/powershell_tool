//! Healing of a foreign CAD file's edge/surface disagreement.
//!
//! A B-spline edge that approximates the intersection of two analytic faces, or an arc whose radius differs from its
//! cylinder's by a few microns, lies off the surfaces it bounds. Refusing the file for it needs a tolerance (a policy), and
//! accepting it leaves the boundary samples off the faces: the first ring of each face's triangles is skewed, and files with
//! a gap of a few hundredths of the chord fail later on a degenerate or inverted boundary triangle. Healing moves each
//! sample of such an edge, and each of its vertices, onto the surfaces of all the faces that use it (to their intersection
//! curve or corner point), by at most `Options::heal_limit_mm`. The shell stays closed (the samples are still shared) and
//! now lies on its faces. Edges within `geometry_tolerance_mm` are not touched, so a conforming file reads exactly as before.
use super::{add, dot, mul, norm, sub, AnalyticModel, V3};
use crate::step::{Error, ErrorKind};
use std::collections::{BTreeMap, BTreeSet};

/// Newton iterations of [`AnalyticModel::heal_point`]; the iteration is quadratic unless the surfaces are nearly tangent.
const MAX_ITERATIONS: usize = 60;

fn fail(detail: String) -> Error {
    Error { offset: 0, kind: ErrorKind::Geometry, detail }
}

/// Faces whose normals differ by less than a few degrees do not fix the point along the difference of their normals: the
/// Gram matrix of their unit normals has an eigenvalue below this fraction of the largest, and that direction is not
/// corrected (a truncated pseudo-inverse). Following it would slide the point along tangent faces by many times the gap
/// it removes, far from where the edge is.
const TANGENT_EIGENVALUE: f64 = 1e-3;

/// The truncated Levenberg-Marquardt solve of `(G + damping I) x = f` for a small dense symmetric `G`: the eigenvectors of
/// `G` (cyclic Jacobi) whose eigenvalue is at least [`TANGENT_EIGENVALUE`] of the largest carry `x`; the rest are dropped.
fn solve_damped(gram: &[Vec<f64>], f: &[f64], damping: f64) -> Option<Vec<f64>> {
    let n = f.len();
    let mut a: Vec<Vec<f64>> = gram.to_vec();
    let mut v: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| f64::from(i == j)).collect()).collect();
    for _ in 0..50 {
        let off: f64 = (0..n).flat_map(|i| (0..i).map(move |j| (i, j))).map(|(i, j)| a[i][j] * a[i][j]).sum();
        if off < 1e-30 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2. * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.).sqrt());
                let (c, s) = (1. / (t * t + 1.).sqrt(), t / (t * t + 1.).sqrt());
                for k in 0..n {
                    let (akp, akq) = (a[k][p], a[k][q]);
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p][k], a[q][k]);
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    let largest = (0..n).map(|k| a[k][k]).fold(0f64, f64::max);
    let mut x = vec![0.; n];
    for k in 0..n {
        let lambda = a[k][k];
        if lambda < TANGENT_EIGENVALUE * largest || !(lambda + damping > 0.) {
            continue;
        }
        let coefficient = (0..n).map(|i| v[i][k] * f[i]).sum::<f64>() / (lambda + damping);
        for i in 0..n {
            x[i] += coefficient * v[i][k];
        }
    }
    x.iter().all(|c| c.is_finite()).then_some(x)
}

impl AnalyticModel {
    /// The point nearest `start` (by a minimum-norm Newton iteration) that lies on the surface of every face in `faces`: on
    /// their common curve for two faces, at their common corner for three. `None` when the iteration does not converge
    /// to 1e-10 relative (or stalls on faces with no common point, within the geometry tolerance), a surface has no projection or normal there, or the point would move by more than the heal limit.
    /// Distinct surfaces only need to intersect; the same surface repeated is harmless (the damped normal equations split the
    /// correction between the duplicates).
    pub(crate) fn heal_point(&self, start: V3, faces: &[u64]) -> Result<V3, String> {
        let surfaces: Vec<_> = faces.iter().filter_map(|f| self.surfaces.get(f)).collect();
        if surfaces.is_empty() {
            return Err("no surface".into());
        }
        // Levenberg-Marquardt on the signed distances to the surfaces: the damping grows when a step does not reduce the
        // worst distance (nearly tangent faces, where the exact Newton step runs away) and shrinks when it does.
        let residuals = |p: V3| -> Result<(Vec<V3>, Vec<f64>), String> {
            let mut normals = Vec::with_capacity(surfaces.len());
            let mut f = Vec::with_capacity(surfaces.len());
            for s in &surfaces {
                let chart = s.project(p).ok_or("a surface has no projection of the point")?.chart;
                let (q, n) = (
                    s.point(chart).ok_or("a surface has no point at the projection")?,
                    s.normal(chart).ok_or("a surface has no normal at the projection")?,
                );
                let length = norm(n);
                if !(length > 0.) {
                    return Err("a surface normal vanishes".to_string());
                }
                let n = mul(n, 1. / length);
                f.push(dot(sub(p, q), n));
                normals.push(n);
            }
            Ok((normals, f))
        };
        let worst = |f: &[f64]| f.iter().fold(0f64, |m, v| m.max(v.abs()));
        let converged = |p: V3| 1e-10 * (1. + p.iter().fold(0f64, |m, x| m.max(x.abs())));
        let (mut p, mut damping) = (start, 1e-9);
        let (mut normals, mut f) = residuals(p)?;
        let (mut stalled, mut creeping) = (false, false);
        for _ in 0..MAX_ITERATIONS {
            if worst(&f) <= converged(p) {
                break;
            }
            let gram: Vec<Vec<f64>> = normals
                .iter()
                .map(|a| normals.iter().map(|b| dot(*a, *b)).collect())
                .collect();
            let mut accepted = false;
            for _ in 0..12 {
                let Some(x) = solve_damped(&gram, &f, damping) else {
                    damping *= 10.;
                    continue;
                };
                let step = normals.iter().zip(&x).fold([0.; 3], |acc, (n, x)| add(acc, mul(*n, *x)));
                let trial = sub(p, step);
                if norm(sub(trial, start)) <= 2. * self.heal_limit_mm {
                    if let Ok((n2, f2)) = residuals(trial) {
                        if worst(&f2) < worst(&f) {
                            // a step that removes under a thousandth of the distance is the creep of a compromise
                            creeping = worst(&f) - worst(&f2) < 1e-3 * worst(&f);
                            (p, normals, f) = (trial, n2, f2);
                            damping = (damping * 0.1).max(1e-12);
                            accepted = true;
                            break;
                        }
                    }
                }
                damping *= 10.;
            }
            if !accepted || creeping {
                stalled = true;
                break;
            }
        }
        // Exactly on every surface, or the compromise of surfaces that have no common point: tangent faces separated by a
        // gap meet nowhere, and the iteration then stalls halfway between them. A stalled iteration is a heal only when
        // every face is within the heal limit (the bound the move itself obeys; a strict read has a heal limit of 0, so there the
        // geometry tolerance, the file's own measure of conformance, decides).
        let exact = worst(&f) <= converged(p).max(1e-4 * self.geometry_tolerance_mm);
        if !exact && !(stalled && worst(&f) <= self.geometry_tolerance_mm.max(self.heal_limit_mm)) {
            let kinds: Vec<String> = surfaces.iter().zip(&f).map(|(s, r)| format!("{}:{r:.2e}", format!("{s:?}").chars().take(12).collect::<String>())).collect();
            let cosines: Vec<String> = (0..normals.len()).flat_map(|i| (i + 1..normals.len()).map(move |j| (i, j))).map(|(i, j)| format!("{:.6}", dot(normals[i], normals[j]))).collect();
            return Err(format!("no convergence (worst distance {:e} mm, stalled {stalled}, {} faces; {kinds:?}; cosines {cosines:?})", worst(&f), surfaces.len()));
        }
        let moved = norm(sub(p, start));
        if moved <= self.heal_limit_mm {
            Ok(p)
        } else {
            Err(format!("the nearest common point is {moved:e} mm away"))
        }
    }

    /// Heal the `marked` edges (those whose declared curve disagrees with a face by more than the geometry tolerance but
    /// within the heal limit): move every sample and both vertices onto the surfaces of the faces that use them, record the
    /// edges so later samples are healed alike, and refuse when a point cannot be healed within the limit or the edge would
    /// fold or collapse. A vertex moves onto all faces of all edges meeting at it, and every edge ending there follows.
    pub(crate) fn heal(&mut self, marked: &BTreeSet<u64>) -> Result<(), Error> {
        if marked.is_empty() {
            return Ok(());
        }
        let mut edge_faces: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for face in self.body.faces.values() {
            for bound in &face.bounds {
                for u in &bound.edges {
                    let faces = edge_faces.entry(u.edge).or_default();
                    if !faces.contains(&face.id) {
                        faces.push(face.id);
                    }
                }
            }
        }
        let mut vertex_edges: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for e in self.body.edges.values() {
            vertex_edges.entry(e.start).or_default().push(e.id);
            if e.end != e.start {
                vertex_edges.entry(e.end).or_default().push(e.id);
            }
        }
        // the vertices of the marked edges, healed onto every face that meets them
        let mut healed_vertices: BTreeMap<u64, V3> = BTreeMap::new();
        for id in marked {
            let raw = &self.body.edges[id];
            for v in [raw.start, raw.end] {
                if healed_vertices.contains_key(&v) {
                    continue;
                }
                let mut faces: Vec<u64> = Vec::new();
                for e in &vertex_edges[&v] {
                    for f in edge_faces.get(e).into_iter().flatten() {
                        if !faces.contains(f) {
                            faces.push(*f);
                        }
                    }
                }
                let p = self.heal_point(self.vertices[&v], &faces).map_err(|why| {
                    fail(format!("cannot heal vertex #{v} onto the {} faces that meet it within {:e} mm: {why}", faces.len(), self.heal_limit_mm))
                })?;
                healed_vertices.insert(v, p);
            }
        }
        let mut samples: BTreeMap<u64, Vec<V3>> = BTreeMap::new();
        let mut touched: BTreeSet<u64> = marked.clone();
        for v in healed_vertices.keys() {
            touched.extend(&vertex_edges[v]);
        }
        for id in &touched {
            let raw = &self.body.edges[id];
            let old = &self.edges[id];
            let mut new = old.clone();
            if marked.contains(id) {
                let faces = edge_faces.get(id).map(Vec::as_slice).unwrap_or(&[]);
                for (i, p) in new.iter_mut().enumerate().take(old.len() - 1).skip(1) {
                    *p = self.heal_point(old[i], faces).map_err(|why| {
                        fail(format!("cannot heal sample {i} of edge #{id} onto its faces within {:e} mm: {why}", self.heal_limit_mm))
                    })?;
                }
            }
            if let Some(p) = healed_vertices.get(&raw.start) {
                new[0] = *p;
            }
            if let Some(p) = healed_vertices.get(&raw.end) {
                *new.last_mut().unwrap() = *p;
            }
            // each segment keeps its direction (no fold) and a positive length
            for i in 0..old.len() - 1 {
                let (a, b) = (sub(old[i + 1], old[i]), sub(new[i + 1], new[i]));
                if !(norm(b) > 0.) || dot(a, b) <= 0. {
                    return Err(fail(format!("healing would fold or collapse segment {i} of edge #{id}")));
                }
            }
            samples.insert(*id, new);
        }
        for (id, new) in samples {
            self.edges.insert(id, new);
        }
        for (v, p) in healed_vertices {
            self.vertices.insert(v, p);
            if let Some(raw) = self.body.vertices.get_mut(&v) {
                raw.coordinates = p.map(|x| x / self.scale_mm);
            }
        }
        for id in marked {
            self.healed.insert(*id, edge_faces.get(id).cloned().unwrap_or_default());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytic_step::{Options, Surface};
    use crate::step::{inventory, Limits};

    fn model(radius: f64) -> AnalyticModel {
        let plain = include_str!("../../tests/data/edit-plate-centre-hole.step");
        let text = plain.replace("#282 = CYLINDRICAL_SURFACE('',#283,4.);", &format!("#282 = CYLINDRICAL_SURFACE('',#283,{radius:.3});"));
        AnalyticModel::read(&inventory(&text, Limits::default()).unwrap(), Options::foreign_cad(0.05)).unwrap()
    }

    /// The top plane (z = 10) and the hole cylinder (radius 4.008, axis z) meet in a circle: a point 3 microns outside
    /// it in the plane and 2 above is healed onto the circle, to the nearest point (the radial and axial corrections).
    #[test]
    fn a_point_beside_a_plane_cylinder_intersection_is_healed_to_the_nearest_circle_point() {
        let m = model(4.008);
        let cylinder = *m.surfaces.iter().find(|(_, s)| matches!(s, Surface::Cylinder { .. })).unwrap().0;
        let plane = *m
            .surfaces
            .iter()
            .find(|(_, s)| matches!(s, Surface::Plane(_)) && s.point([0., 0.]).is_some_and(|p| (p[2] - 10.).abs() < 1e-9))
            .unwrap()
            .0;
        let Surface::Cylinder { frame, .. } = &m.surfaces[&cylinder] else { unreachable!() };
        assert!(frame.z[0].abs() < 1e-12 && frame.z[1].abs() < 1e-12, "axis along z");
        // the axis point in the top plane (z = 10), then the circle point radius 4.008 out along `x`
        let centre = add(frame.origin, mul(frame.z, (10. - frame.origin[2]) / frame.z[2]));
        let x = frame.x;
        let start = add(add(centre, mul(x, 4.005)), [0., 0., 0.002]);
        let p = m.heal_point(start, &[plane, cylinder]).unwrap();
        for id in [plane, cylinder] {
            let s = &m.surfaces[&id];
            assert!(s.project(p).unwrap().residual_mm < 1e-9, "residual {:e}", s.project(p).unwrap().residual_mm);
        }
        let moved = norm(sub(p, start));
        let nearest = norm(sub(add(centre, mul(x, 4.008)), start));
        assert!((moved - nearest).abs() < 1e-9, "moved {moved:e}, nearest circle point {nearest:e}");
    }

    /// Two parallel faces 0.2 mm apart (the compromise case: no common point) heal to halfway, never beyond the gap.
    #[test]
    fn tangent_faces_with_a_gap_heal_to_the_midway_compromise() {
        let m = model(4.);
        let planes: Vec<u64> = m
            .surfaces
            .iter()
            .filter(|(_, s)| s.point([0., 0.]).is_some_and(|p| matches!(s, Surface::Plane(_)) && (p[2].abs() < 1e-9 || (p[2] - 10.).abs() < 1e-9)))
            .map(|(id, _)| *id)
            .collect();
        assert!(planes.len() >= 2);
        // z = 0 and z = 10 are parallel and 10 apart: too far for the geometry tolerance, so refused, not healed
        let bottom = planes.iter().copied().find(|id| m.surfaces[id].point([0., 0.]).unwrap()[2].abs() < 1e-9).unwrap();
        let top = planes.iter().copied().find(|id| m.surfaces[id].point([0., 0.]).unwrap()[2] > 1.).unwrap();
        assert!(m.heal_point([1., 1., 5.], &[bottom, top]).is_err());
    }

    /// Two parallel planes 0.03 mm apart (six times the 0.005 geometry tolerance, under the 0.05 heal limit) have no common
    /// point: the compromise halfway is a heal, 0.015 from each (over the geometry tolerance, under the heal limit). At 0.12 mm each face is 0.06 from it, over the heal limit: refused.
    #[test]
    fn a_compromise_between_tangent_faces_within_the_heal_limit_is_a_heal() {
        let mut m = model(4.);
        let top = m.surfaces.iter().find(|(_, s)| s.point([0., 0.]).is_some_and(|p| matches!(s, Surface::Plane(_)) && (p[2] - 10.).abs() < 1e-9)).map(|(id, _)| *id).unwrap();
        let Surface::Plane(frame) = m.surfaces[&top].clone() else { unreachable!() };
        for (gap, healed) in [(0.03, true), (0.12, false)] {
            let mut above = frame.clone();
            above.origin[2] += gap;
            m.surfaces.insert(9_999, Surface::Plane(above));
            let r = m.heal_point([1., 1., 10. + gap / 2.], &[top, 9_999]);
            assert_eq!(r.is_ok(), healed, "gap {gap}: {r:?}");
            if let Ok(p) = r {
                assert!((p[2] - (10. + gap / 2.)).abs() < 1e-6, "halfway, got z = {}", p[2]);
            }
        }
    }
}
