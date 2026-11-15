//! The hybrid mesher on outlines that are not rectilinear: a deterministic sweep over random convex polygons, discs,
//! chamfered and sheared plates, with and without a hole, in both element families. Every mesh must be conforming, free of
//! inverted elements and cover the region; the share of cases that stay hybrid (instead of falling back, explicitly) must stay
//! high, because that is what the boundary layer exists for.

use fea_core::geometry::{Loop, Region};
use fea_core::mesh::{Elastic, Physics};
use fea_core::meshing::hybrid::{mesh_hybrid_2d, ElementFamily, HybridOptions2d, Strategy};

/// Small deterministic generator (the sweep must not change between runs).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / ((1u64 << 31) as f64)
    }
}

fn outline(rng: &mut Lcg) -> Loop {
    match (rng.next() * 4.0) as usize {
        0 => {
            // random convex polygon on an ellipse, rotated
            let n = 3 + (rng.next() * 6.0) as usize;
            let rot = rng.next() * std::f64::consts::TAU;
            let (a, b) = (3.0 + rng.next() * 6.0, 3.0 + rng.next() * 6.0);
            let pts: Vec<[f64; 2]> = (0..n).map(|k| {
                let th = rot + std::f64::consts::TAU * k as f64 / n as f64;
                [a * th.cos(), b * th.sin()]
            }).collect();
            Loop::polygon(&pts, "edge").unwrap()
        }
        1 => Loop::circle([0.0, 0.0], 3.0 + rng.next() * 5.0, "rim").unwrap(),
        2 => {
            // rectangle with a chamfered corner
            let (w, h, c) = (6.0 + rng.next() * 8.0, 3.0 + rng.next() * 5.0, 0.5 + rng.next() * 1.5);
            Loop::polygon(&[[0.0, 0.0], [w, 0.0], [w, h - c], [w - c, h], [0.0, h]], "edge").unwrap()
        }
        _ => {
            // parallelogram
            let (w, h, sk) = (8.0 + rng.next() * 6.0, 2.0 + rng.next() * 4.0, rng.next() * 3.0);
            Loop::polygon(&[[0.0, 0.0], [w, 0.0], [w + sk, h], [sk, h]], "edge").unwrap()
        }
    }
}

#[test]
fn angled_and_curved_outlines_mesh_as_conforming_hybrids_or_fall_back_with_a_reason() {
    let steel = Elastic::new(30e6, 0.3);
    let mut rng = Lcg(12345);
    let (mut cases, mut hybrid) = (0, 0);
    for case in 0..120 {
        let outer = outline(&mut rng);
        let (lo, hi) = Region::new(outer.clone(), vec![], steel).unwrap().bounds();
        let mut holes = vec![];
        if rng.next() < 0.5 {
            let c = [lo[0] + (hi[0] - lo[0]) * (0.35 + 0.3 * rng.next()), lo[1] + (hi[1] - lo[1]) * (0.35 + 0.3 * rng.next())];
            holes.push(Loop::circle(c, 0.2 + 0.5 * rng.next(), "hole").unwrap());
        }
        let Ok(region) = Region::new(outer, holes, steel) else { continue };
        let size = 0.15 + rng.next() * 0.5;
        let family = if rng.next() < 0.5 { ElementFamily::Linear } else { ElementFamily::Quadratic };
        let h = mesh_hybrid_2d(&region, &|_| size, &HybridOptions2d::new(Physics::PlaneStress { thickness: 1.0 }, family)).unwrap_or_else(|e| panic!("case {case}: {e}"));
        cases += 1;
        let r = &h.report;
        assert!(r.conformity.is_conforming() && r.conformity.components == 1, "case {case}: {}", r.conformity.summary());
        assert_eq!(r.quality.inverted, 0, "case {case}");
        match &r.strategy {
            Strategy::Hybrid => {
                hybrid += 1;
                assert!(r.structured_elements > 0 && r.unstructured_elements > 0, "case {case}: a hybrid mesh has both families");
            }
            Strategy::Unstructured { reason } => assert!(reason.contains("regular"), "case {case}: unexpected fallback: {reason}"),
        }
    }
    // measured: 117 of 120; the rest are regions too small for the element size to leave any regular area
    assert!(hybrid * 10 >= cases * 8, "only {hybrid} of {cases} outlines stayed hybrid");
}
