//! One call that makes a sampled shell fit for constrained recovery: weld near-degenerate short edges, then dissolve
//! redundant flat and collinear vertices. Each stage is optional in effect: a stage that refuses (open shell, failed
//! verification) leaves the previous shell untouched and says why in [`Prepared::notes`]. The smallest shell angle before and
//! after is what the compatibility registry gates the native constrained strategy on.

use crate::{
    plc_conform::min_shell_angle_deg,
    shell_defeature::{self, defeature_slivers},
    shell_patch::{self, dissolve_flat},
    shell_refine::{self, refine_surface},
    shell_weld::{self, weld_short_edges},
    V3,
};

#[derive(Debug, Clone)]
pub struct Prepared {
    pub vertices: Vec<V3>,
    pub facets: Vec<[usize; 3]>,
    /// Smallest input angle (corner or dihedral, degrees) before and after preparation.
    pub angle_before_deg: f64,
    pub angle_after_deg: f64,
    pub weld: Option<shell_weld::Report>,
    pub defeature: Option<shell_defeature::Report>,
    pub cleanup: Option<shell_patch::Report>,
    pub refine: Option<shell_refine::Report>,
    /// Why a stage did not run, in order.
    pub notes: Vec<String>,
}

/// Prepare a closed outward-oriented triangulated shell. Never fails: the worst case returns the input unchanged.
pub fn prepare_shell(
    vertices: &[V3],
    facets: &[[usize; 3]],
    weld_options: shell_weld::Options,
    cleanup_options: shell_patch::Options,
) -> Prepared {
    prepare_shell_with(vertices, facets, weld_options, cleanup_options, None)
}

/// [`prepare_shell`] with the optional last stage: shape-preserving surface Steiner refinement (`shell_refine`). It is opt-in
/// because it adds vertices (a budget of `max_new_vertex_factor` times the shell's) and cannot raise the smallest input angle
/// above what the faces' own corners allow.
pub fn prepare_shell_with(
    vertices: &[V3],
    facets: &[[usize; 3]],
    weld_options: shell_weld::Options,
    cleanup_options: shell_patch::Options,
    refine_options: Option<shell_refine::Options>,
) -> Prepared {
    let angle_before = min_shell_angle_deg(vertices, facets);
    let mut notes = Vec::new();
    let (mut v, mut f) = (vertices.to_vec(), facets.to_vec());
    let weld = match weld_short_edges(&v, &f, weld_options) {
        Ok(w) => {
            let report = w.report.clone();
            v = w.vertices;
            f = w.facets;
            Some(report)
        }
        Err(e) => {
            notes.push(format!("weld skipped: {e}"));
            None
        }
    };
    // Sliver faces: clusters of needle-edge vertices welded together (the collapse above cannot, by its link condition).
    let defeature = match defeature_slivers(&v, &f, Default::default()) {
        Ok(d) => {
            let report = d.report.clone();
            v = d.vertices;
            f = d.facets;
            Some(report)
        }
        Err(e) => {
            notes.push(format!("defeaturing skipped: {e}"));
            None
        }
    };
    // The weld and the defeaturing may have moved geometry by up to its longest collapsed edge, so planes and straight lines are only that
    // exact afterwards: the cleanup must accept deviations of that size, or it would not recognise what the weld left flat.
    let mut cleanup_options = cleanup_options;
    let moved = weld
        .iter()
        .map(|w| w.longest_collapsed_edge)
        .chain(defeature.iter().map(|d| d.longest_weld))
        .fold(0., f64::max);
    if moved > 0. {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in &v {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let diag = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt();
        if diag > 0. {
            let scaled = 2. * moved / diag;
            cleanup_options.relative_tolerance = cleanup_options.relative_tolerance.max(scaled);
            cleanup_options.collinear_distance = cleanup_options.collinear_distance.max(scaled);
        }
    }
    let cleanup = match dissolve_flat(&v, &f, cleanup_options) {
        Ok(c) => {
            let report = c.report.clone();
            v = c.vertices;
            f = c.facets;
            Some(report)
        }
        Err(e) => {
            notes.push(format!("flat-patch cleanup skipped: {e}"));
            None
        }
    };
    // Surface Steiner points (opt-in): chords across coarsely sampled arcs cannot be fixed with the existing vertices.
    let refine = match refine_options {
        None => None,
        Some(options) => match refine_surface(&v, &f, options) {
            Ok(r) => {
                let report = r.report.clone();
                v = r.vertices;
                f = r.facets;
                Some(report)
            }
            Err(e) => {
                notes.push(format!("surface refinement skipped: {e}"));
                None
            }
        },
    };
    // Never hand back a shell with a worse input angle than the one supplied.
    let angle_after = min_shell_angle_deg(&v, &f);
    if angle_after < angle_before * (1. - 1e-9) {
        notes.push(format!(
            "preparation lowered the smallest shell angle ({angle_before:.4} -> {angle_after:.4} degrees); the input is returned unchanged"
        ));
        return Prepared {
            vertices: vertices.to_vec(),
            facets: facets.to_vec(),
            angle_before_deg: angle_before,
            angle_after_deg: angle_before,
            weld,
            defeature,
            cleanup,
            refine,
            notes,
        };
    }
    Prepared {
        vertices: v,
        facets: f,
        angle_before_deg: angle_before,
        angle_after_deg: angle_after,
        weld,
        defeature,
        cleanup,
        refine,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_patch::tests::grid_box;

    #[test]
    fn a_gridded_box_with_a_seam_needle_prepares_to_a_healthy_shell() {
        let (mut v, f) = grid_box(4, 4, 1);
        // Pull a top-edge point to 2e-4 from its end and 2e-6 out of the wall plane.
        let i = v.iter().position(|p| *p == [1., 0., 1.]).unwrap();
        v[i] = [2e-4, -2e-6, 1.];
        let p = prepare_shell(&v, &f, Default::default(), Default::default());
        assert!(p.angle_before_deg < 0.1, "{p:?}");
        assert!(p.angle_after_deg > 14., "{p:?}");
        assert!(p.vertices.len() <= 8 + 1, "{} vertices", p.vertices.len());
        assert!(p.weld.is_some() && p.cleanup.is_some() && p.notes.is_empty());
    }

    #[test]
    fn an_open_shell_comes_back_unchanged_with_notes() {
        let (v, mut f) = grid_box(2, 2, 1);
        f.pop();
        let p = prepare_shell(&v, &f, Default::default(), Default::default());
        assert_eq!(p.facets, f);
        assert_eq!(p.vertices, v);
        assert_eq!(p.notes.len(), 3, "{:?}", p.notes);
    }

    #[test]
    fn a_healthy_shell_is_returned_as_it_is() {
        let (v, f) = grid_box(1, 1, 1);
        let p = prepare_shell(&v, &f, Default::default(), Default::default());
        assert_eq!(p.facets, f);
        assert!((p.angle_after_deg - p.angle_before_deg).abs() < 1e-12);
    }
}
