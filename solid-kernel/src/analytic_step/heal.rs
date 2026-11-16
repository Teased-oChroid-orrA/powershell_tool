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

/// Solve `(G + damping I) x = f` for a small dense symmetric `G` by Gaussian elimination with partial pivoting.
fn solve_damped(gram: &[Vec<f64>], f: &[f64], damping: f64) -> Option<Vec<f64>> {
    let n = f.len();
    let mut a: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            let mut row = gram[i].clone();
            row[i] += damping;
            row.push(f[i]);
            row
        })
        .collect();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if !(a[pivot][col].abs() > 1e-300) {
            return None;
        }
        a.swap(col, pivot);
        for row in col + 1..n {
            let m = a[row][col] / a[col][col];
            for k in col..=n {
                a[row][k] -= m * a[col][k];
            }
        }
    }
    let mut x = vec![0.; n];
    for i in (0..n).rev() {
        let tail: f64 = (i + 1..n).map(|k| a[i][k] * x[k]).sum();
        x[i] = (a[i][n] - tail) / a[i][i];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

impl AnalyticModel {
    /// The point nearest `start` (by a minimum-norm Newton iteration) that lies on the surface of every face in `faces`: on
    /// their common curve for two faces, at their common corner for three. `None` when the iteration does not converge
    /// to 1e-10 relative, a surface has no projection or normal there, or the point would move by more than the heal limit.
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
                            (p, normals, f) = (trial, n2, f2);
                            damping = (damping * 0.1).max(1e-12);
                            accepted = true;
                            break;
                        }
                    }
                }
                damping *= 10.;
            }
            if !accepted {
                break;
            }
        }
        // a point on the surfaces to a nanometre-scale fraction of the geometry tolerance is healed
        if worst(&f) > converged(p).max(1e-4 * self.geometry_tolerance_mm) {
            return Err(format!("no convergence (worst distance {:e} mm, {} faces)", worst(&f), surfaces.len()));
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
