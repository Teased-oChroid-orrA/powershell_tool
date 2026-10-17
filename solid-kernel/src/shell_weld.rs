//! Weld of near-degenerate short edges of a closed, outward-oriented triangulated shell.
//!
//! Sampling a CAD shell leaves vertices a hair away from their neighbours, typically along a tangent seam between a flat and a
//! curved face: a vertex 5e-3 from its neighbour and 4e-6 off the line through the others. Triangles spanned by them are
//! needles, they cannot be dissolved by re-triangulating one flat patch (`shell_patch`) because the curved side shares the
//! vertex, and every needle edge is a protected segment for constrained recovery. This pass collapses such short edges on the
//! shell itself:
//!
//! * candidates: edges no longer than `max_edge_relative` x bounding-box diagonal that belong to a triangle whose smallest
//!   corner angle is below `bad_angle_deg`, thinnest triangle first;
//! * a collapse `u -> v` (removing `u`) must satisfy the link condition (the only common neighbours of `u` and `v` are the
//!   two apexes of the edge), keep every changed triangle's normal within `max_normal_change_deg` of its old one with positive
//!   area, and not push the smallest corner angle of the changed triangles below `min(old minimum, acceptable_angle_deg)`;
//!   both directions are tried and the better one kept;
//! * collapses of one round touch disjoint stars; rounds repeat until nothing changes.
//!
//! The geometry moves by at most the length of a collapsed edge, so the signed volume may change by at most four times that
//! length times the surface area, which is what the final verification allows. The result must be a closed oriented manifold.

use crate::shell_patch::{
    area, check_closed, cross, dot, min_corner_angle_deg, norm, signed_volume, sub,
};
use crate::V3;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Longest edge considered, as a fraction of the bounding-box diagonal.
    pub max_edge_relative: f64,
    /// Only edges of triangles with a corner thinner than this are candidates.
    pub bad_angle_deg: f64,
    /// A changed triangle may not turn by more than this.
    pub max_normal_change_deg: f64,
    /// A collapse may reduce the smallest changed angle, but never below `min(old, acceptable_angle_deg)`.
    pub acceptable_angle_deg: f64,
    pub max_rounds: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_edge_relative: 1e-3,
            bad_angle_deg: 5.,
            max_normal_change_deg: 30.,
            acceptable_angle_deg: 10.,
            max_rounds: 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rounds: usize,
    pub collapses: usize,
    pub longest_collapsed_edge: f64,
    /// Candidate collapses refused, by reason (a candidate is counted once per direction tried).
    pub rejected_link: usize,
    pub rejected_normal: usize,
    pub rejected_angle: usize,
    pub rejected_busy: usize,
    pub vertices_before: usize,
    pub vertices_after: usize,
    pub facets_before: usize,
    pub facets_after: usize,
    pub min_angle_before_deg: f64,
    pub min_angle_after_deg: f64,
}

#[derive(Debug, Clone)]
pub struct Welded {
    pub vertices: Vec<V3>,
    pub facets: Vec<[usize; 3]>,
    pub report: Report,
}

fn tri_min_angle(vertices: &[V3], f: [usize; 3]) -> f64 {
    min_corner_angle_deg(vertices, &[f])
}

/// Collapse near-degenerate short edges. See the module documentation.
pub fn weld_short_edges(
    vertices: &[V3],
    facets: &[[usize; 3]],
    options: Options,
) -> Result<Welded, String> {
    check_closed(facets)?;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for v in vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let diag = norm(sub(hi, lo));
    if !(diag.is_finite() && diag > 0.) {
        return Err("degenerate bounding box".into());
    }
    let max_len = options.max_edge_relative * diag;
    let cos_limit = options.max_normal_change_deg.to_radians().cos();
    let verts: Vec<V3> = vertices.to_vec();
    let mut tris: Vec<[usize; 3]> = facets.to_vec();
    let mut alive = vec![true; verts.len()];
    let before_angle = min_corner_angle_deg(&verts, &tris);
    let volume_before = signed_volume(&verts, &tris);
    let surface: f64 = tris.iter().map(|&f| area(&verts, f)).sum();
    let mut report = Report {
        rounds: 0,
        collapses: 0,
        longest_collapsed_edge: 0.,
        rejected_link: 0,
        rejected_normal: 0,
        rejected_angle: 0,
        rejected_busy: 0,
        vertices_before: verts.len(),
        vertices_after: verts.len(),
        facets_before: tris.len(),
        facets_after: tris.len(),
        min_angle_before_deg: before_angle,
        min_angle_after_deg: before_angle,
    };
    for _ in 0..options.max_rounds {
        let mut star: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (t, f) in tris.iter().enumerate() {
            for &v in f {
                star.entry(v).or_default().push(t);
            }
        }
        let mut cands: Vec<(f64, f64, [usize; 2])> = Vec::new();
        for f in &tris {
            let a = tri_min_angle(&verts, *f);
            if a >= options.bad_angle_deg {
                continue;
            }
            for k in 0..3 {
                let (u, v) = (f[k], f[(k + 1) % 3]);
                let len = norm(sub(verts[u], verts[v]));
                if len <= max_len {
                    cands.push((a, len, [u.min(v), u.max(v)]));
                }
            }
        }
        cands.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        cands.dedup_by(|a, b| a.2 == b.2);
        let mut dead = vec![false; tris.len()];
        let mut touched: BTreeSet<usize> = BTreeSet::new();
        let mut added: Vec<[usize; 3]> = Vec::new();
        let mut done = 0;
        for (_, len, [a, b]) in cands {
            if !alive[a] || !alive[b] {
                continue;
            }
            let mut best: Option<(f64, usize, usize, Vec<[usize; 3]>)> = None;
            for (u, v) in [(a, b), (b, a)] {
                if touched.contains(&u) || touched.contains(&v) {
                    report.rejected_busy += 1;
                    continue;
                }
                let su = &star[&u];
                let sv = &star[&v];
                if su.iter().chain(sv.iter()).any(|&t| dead[t]) {
                    continue;
                }
                let around: BTreeSet<usize> =
                    su.iter().chain(sv.iter()).flat_map(|&t| tris[t]).collect();
                if around.iter().any(|w| touched.contains(w)) {
                    report.rejected_busy += 1;
                    continue;
                }
                // Triangles holding the edge, and their apexes.
                let on_edge: Vec<usize> = su
                    .iter()
                    .copied()
                    .filter(|&t| tris[t].contains(&v))
                    .collect();
                if on_edge.len() != 2 {
                    continue;
                }
                let apex: BTreeSet<usize> = on_edge
                    .iter()
                    .flat_map(|&t| tris[t])
                    .filter(|&w| w != u && w != v)
                    .collect();
                let nu: BTreeSet<usize> = su
                    .iter()
                    .flat_map(|&t| tris[t])
                    .filter(|&w| w != u)
                    .collect();
                let nv: BTreeSet<usize> = sv
                    .iter()
                    .flat_map(|&t| tris[t])
                    .filter(|&w| w != v)
                    .collect();
                let common: BTreeSet<usize> = nu.intersection(&nv).copied().collect();
                if common != apex {
                    report.rejected_link += 1;
                    continue; // link condition
                }
                let changed: Vec<usize> = su
                    .iter()
                    .copied()
                    .filter(|t| !on_edge.contains(t))
                    .collect();
                let mut new_tris = Vec::with_capacity(changed.len());
                let mut old_min = f64::INFINITY;
                let mut new_min = f64::INFINITY;
                let mut ok = true;
                for &t in &changed {
                    let old = tris[t];
                    let new = old.map(|w| if w == u { v } else { w });
                    let (no, nn) = (
                        cross(
                            sub(verts[old[1]], verts[old[0]]),
                            sub(verts[old[2]], verts[old[0]]),
                        ),
                        cross(
                            sub(verts[new[1]], verts[new[0]]),
                            sub(verts[new[2]], verts[new[0]]),
                        ),
                    );
                    let (lo_, ln_) = (norm(no), norm(nn));
                    if ln_.is_nan()
                        || ln_ <= 0.
                        || lo_ <= 0.
                        || dot(no, nn) / (lo_ * ln_) < cos_limit
                    {
                        ok = false;
                        break;
                    }
                    old_min = old_min.min(tri_min_angle(&verts, old));
                    new_min = new_min.min(tri_min_angle(&verts, new));
                    new_tris.push(new);
                }
                if !ok || new_tris.is_empty() {
                    report.rejected_normal += 1;
                    continue;
                }
                if new_min < old_min.min(options.acceptable_angle_deg) * (1. - 1e-9) {
                    report.rejected_angle += 1;
                    continue;
                }
                if best.as_ref().is_none_or(|b| new_min > b.0) {
                    best = Some((new_min, u, v, new_tris));
                }
            }
            let Some((_, u, v, new_tris)) = best else {
                continue;
            };
            for &t in &star[&u] {
                dead[t] = true;
            }
            touched.extend(star[&u].iter().flat_map(|&t| tris[t]));
            touched.extend(star[&v].iter().flat_map(|&t| tris[t]));
            alive[u] = false;
            added.extend(new_tris);
            report.longest_collapsed_edge = report.longest_collapsed_edge.max(len);
            done += 1;
        }
        report.rounds += 1;
        if done == 0 {
            break;
        }
        report.collapses += done;
        tris = tris
            .iter()
            .zip(&dead)
            .filter(|(_, d)| !**d)
            .map(|(f, _)| *f)
            .chain(added)
            .collect();
    }
    // Compact and verify.
    let mut map = BTreeMap::new();
    let mut out_v = Vec::new();
    let used: BTreeSet<usize> = tris.iter().flatten().copied().collect();
    for (i, v) in verts.iter().enumerate() {
        if used.contains(&i) {
            map.insert(i, out_v.len());
            out_v.push(*v);
        }
    }
    let out_f: Vec<[usize; 3]> = tris.iter().map(|f| f.map(|v| map[&v])).collect();
    check_closed(&out_f).map_err(|e| format!("weld produced an invalid shell: {e}"))?;
    if out_f.iter().any(|&f| area(&out_v, f) <= 0.) {
        return Err("weld produced a zero-area facet".into());
    }
    let volume_after = signed_volume(&out_v, &out_f);
    let allowed = 1e-9 * volume_before.abs() + 4. * report.longest_collapsed_edge * surface;
    if (volume_after - volume_before).abs() > allowed {
        return Err(format!(
            "weld changed the volume by more than the collapsed edge lengths allow: {volume_before} -> {volume_after}"
        ));
    }
    report.vertices_after = out_v.len();
    report.facets_after = out_f.len();
    report.min_angle_after_deg = min_corner_angle_deg(&out_v, &out_f);
    Ok(Welded {
        vertices: out_v,
        facets: out_f,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_patch::tests::grid_box;

    /// A grid box with the top-edge point (1, 0, 1) pulled to `(x, -2e-6, 1)`: the point stays in the top plane but leaves
    /// the wall plane, so no flat patch can dissolve it, and for small `x` every triangle at it is a needle.
    fn seam_needle(x: f64) -> (Vec<V3>, Vec<[usize; 3]>, usize) {
        let (mut v, f) = grid_box(2, 2, 1);
        let i = v
            .iter()
            .position(|p| *p == [1., 0., 1.])
            .expect("edge point exists");
        v[i] = [x, -2e-6, 1.];
        (v, f, i)
    }

    #[test]
    fn a_seam_needle_is_welded_into_its_neighbour_and_the_shell_stays_valid() {
        let (v, f, _) = seam_needle(2e-4);
        let before = min_corner_angle_deg(&v, &f);
        assert!(before < 1.);
        let w = weld_short_edges(&v, &f, Options::default()).unwrap();
        assert!(w.report.collapses >= 1, "{:?}", w.report);
        assert_eq!(w.vertices.len(), v.len() - w.report.collapses);
        assert!(w.report.min_angle_after_deg > 18., "{:?}", w.report);
        assert!((signed_volume(&w.vertices, &w.facets) - 4.).abs() < 1e-3);
        assert!(w.report.longest_collapsed_edge < 1e-3 * 4.0);
        check_closed(&w.facets).unwrap();
    }

    #[test]
    fn healthy_shells_and_long_edges_are_untouched() {
        let (v, f) = grid_box(2, 2, 1);
        let w = weld_short_edges(&v, &f, Options::default()).unwrap();
        assert_eq!(w.report.collapses, 0);
        assert_eq!(w.facets, f);
        // A needle whose short edge is longer than the allowed fraction is left alone.
        let (v, f, _) = seam_needle(0.3);
        let opt = Options {
            max_edge_relative: 1e-6,
            ..Options::default()
        };
        assert_eq!(weld_short_edges(&v, &f, opt).unwrap().report.collapses, 0);
    }

    #[test]
    fn welding_is_deterministic_and_never_worsens_the_minimum_angle() {
        let (v, f, _) = seam_needle(5e-4);
        let a = weld_short_edges(&v, &f, Options::default()).unwrap();
        let b = weld_short_edges(&v, &f, Options::default()).unwrap();
        assert_eq!(a.facets, b.facets);
        assert_eq!(a.vertices, b.vertices);
        assert!(a.report.min_angle_after_deg >= a.report.min_angle_before_deg);
    }

    #[test]
    fn an_open_shell_is_refused() {
        let (v, mut f) = grid_box(1, 1, 1);
        f.pop();
        assert!(weld_short_edges(&v, &f, Options::default()).is_err());
    }
}
