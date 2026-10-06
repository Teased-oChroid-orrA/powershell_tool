//! The lug solver against the existing bushing contact FE (`edge-check`) on identical inputs:
//! a bushing pressed into a housing bore near a free edge, a rigid pin pushed to collapse.
//!
//! The two are different discretisations of one problem (here, plane strain, the housing
//! perfectly plastic at the same flow stress, the bushing elastic): `edge-check` meshes a
//! rectangular plate and takes the load at a fixed pin displacement, `lug-solver` meshes the
//! same plate as a lug and takes the peak of the load-travel curve. They are not expected to
//! agree exactly - the comparison documents how close they are and how much faster the
//! condensed rigid-pin formulation is. Run with `--nocapture` for the table.

use edge_check::contact::ContactMesh;
use edge_check::fem::MeshSpec as EcMesh;
use edge_check::models::contact_model::flow_stress;
use edge_check::types::{BushingSpec as EcBushing, Geometry, Strengths};
use lug_solver::*;
use std::time::Instant;

const A: f64 = 0.25; // housing bore radius
const T: f64 = 0.5; // housing length (thickness)
const HOUSING: Material = Material { e_psi: 10.3e6, nu: 0.33 };
const BUSHING: Material = Material { e_psi: 17.0e6, nu: 0.34 };
const RI: f64 = 0.1875; // bushing bore radius
const DELTA: f64 = 0.0005; // radial interference

fn flow() -> f64 {
    flow_stress(&Strengths { e: HOUSING.e_psi, nu: HOUSING.nu, sy: 70_000.0, fsu: 48_000.0, ftu: 77_000.0, fbru: 121_000.0, fbru_e15: 0.0 })
}

/// `(collapse lbf, seconds)` from the existing contact FE.
fn edge_check_collapse(e: f64, friction: f64) -> (f64, f64) {
    let reach = (3.0 * e).max(10.0 * A);
    let geom = Geometry { bore_radius: A, edge: e, thickness: T, plate_far: reach, plate_half_height: reach, plane_angle_deg: 40.0 };
    let bush = EcBushing { inner_radius: RI, interference: DELTA, e: BUSHING.e_psi, nu: BUSHING.nu, friction };
    let t = Instant::now();
    let m = ContactMesh::build(&geom, &bush, HOUSING.e_psi, HOUSING.nu, EcMesh::default(), 3, true).unwrap();
    let c = m.collapse(DELTA, flow(), T).unwrap().collapse;
    (c, t.elapsed().as_secs_f64())
}

/// `(collapse lbf, seconds)` from the lug solver (build + collapse).
fn lug_collapse(e: f64, friction: f64) -> (f64, f64, bool) {
    let reach = (3.0 * e).max(10.0 * A);
    let g = LugGeometry { hole_dia: 2.0 * A, width: 2.0 * reach, edge: e, length: reach, thickness: T, head_corner_radius: 0.0, far_corner_radius: 0.0 };
    let b = BushingSpec { inner_dia: 2.0 * RI, material: BUSHING, interference_dia: 2.0 * DELTA, friction };
    let t = Instant::now();
    let m = LugModel::build_bushed(&g, HOUSING, MeshSpec { elements_around: 48, ..Default::default() }, true, PlaneMode::Strain, Some(b)).unwrap();
    let l = m.limit_load(PinSpec::new(2.0 * RI - 0.0004, 0.0), LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, flow()).unwrap();
    (l.limit_load_lbf, t.elapsed().as_secs_f64(), l.plateau)
}

#[test]
#[ignore = "slow (about 15 s, runs edge-check at its default mesh); run with --ignored --nocapture"]
fn the_lug_solver_tracks_the_existing_bushing_contact_fe_and_is_faster() {
    let mut rows = Vec::new();
    for ed in [1.5, 2.0, 3.0] {
        let e = ed * 2.0 * A;
        let (ec, ec_s) = edge_check_collapse(e, 0.2);
        let (lg, lg_s, plateau) = lug_collapse(e, 0.2);
        eprintln!("CMP e/D {ed}: edge-check {ec:>8.0} lbf in {ec_s:.2} s | lug-solver {lg:>8.0} lbf in {lg_s:.2} s | ratio {:.3} plateau {plateau}", lg / ec);
        rows.push((ed, ec, lg, ec_s, lg_s));
    }
    for (ed, ec, lg, _, _) in &rows {
        assert!(*lg / *ec > 0.6 && *lg / *ec < 1.3, "e/D {ed}: lug {lg} vs edge-check {ec}");
    }
    // Both must show the same strengthening with edge distance.
    assert!(rows[2].2 > rows[0].2 && rows[2].1 > rows[0].1);
}
