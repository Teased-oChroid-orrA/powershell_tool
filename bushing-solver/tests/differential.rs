//! Differential test against real output from engineering.toolbox's own
//! TypeScript `computeBushing` (the production engine this crate ports)
//! - not a hand-derived expected value. Golden numbers below were
//! captured by running the real TS engine against the project's own
//! `tests/bushing-fixture.ts` `baseBushingInput` (straight/straight mode)
//! via `npx tsx`, on 2026-08-27. Reproduce with:
//!
//! ```sh
//! cd ~/Claude/Projects/engineering.toolbox
//! npx tsx -e "
//!   import('./src/lib/core/bushing').then(({computeBushing}) => {
//!     console.log(JSON.stringify(computeBushing({
//!       units: 'imperial', boreDia: 0.5, idBushing: 0.375, interference: 0.0015,
//!       housingLen: 0.5, housingWidth: 1.5, edgeDist: 0.75,
//!       bushingType: 'straight', idType: 'straight',
//!       csMode: 'depth_angle', csDia: 0.5, csDepth: 0.125, csAngle: 100,
//!       extCsMode: 'depth_angle', extCsDia: 0.625, extCsDepth: 0.125, extCsAngle: 100,
//!       matHousing: 'al7075', matBushing: 'bronze', friction: 0.15, dT: 0,
//!       minWallStraight: 0.05, minWallNeck: 0.04
//!     }), null, 2));
//!   });
//! "
//! ```
//!
//! This specific case is a real, useful edge case, not a hand-picked easy
//! one: with `edgeDist: 0.75`, the governing check is edge-distance
//! sequencing with a *negative* margin (a real fail), not the wall
//! thickness or a trivially-passing case - proving the port gets the
//! failure direction right, not just the happy path.

use bushing_solver::solve::{compute, BushingInputs, EndConstraint};

fn golden_input() -> BushingInputs {
    BushingInputs {
        bore_dia: 0.5,
        bore_tol_plus: 0.0,
        bore_tol_minus: 0.0,
        id_bushing: 0.375,
        interference: 0.0015,
        interference_tol_plus: 0.0,
        interference_tol_minus: 0.0,
        housing_len: 0.5,
        housing_width: 1.5,
        edge_dist: 0.75,
        mat_housing: "al7075".to_string(),
        mat_bushing: "bronze".to_string(),
        friction: Some(0.15),
        d_t: 0.0,
        end_constraint: EndConstraint::Free,
        min_wall_straight: 0.05,
        edge_load_angle_deg: None,
        load: None,
    }
}

/// Relative-or-absolute tolerance compare - the golden values were
/// copy-pasted from JSON with full f64 precision, but this still isn't
/// meant to catch float-formatting noise, just real formula divergence.
fn close(actual: f64, expected: f64, label: &str) {
    let diff = (actual - expected).abs();
    let tol = expected.abs() * 1e-6 + 1e-9;
    assert!(diff <= tol, "{label}: expected {expected}, got {actual} (diff {diff}, tol {tol})");
}

#[test]
fn matches_real_ts_engine_output_for_the_project_base_fixture() {
    let out = compute(&golden_input());

    close(out.od_installed, 0.5015, "od_installed");
    close(out.wall_straight, 0.06324999999999997, "wall_straight");
    close(out.pressure, 8794.147762602435, "pressure");
    close(out.term_b, 9.50420168067227e-8, "term_b");
    close(out.term_h, 7.55259609544394e-8, "term_h");
    close(out.psi, 1.0227546149094484, "psi");
    close(out.lambda, 0.8862269254527579, "lambda");
    close(out.effective_od_housing, 1.692568750643269, "effective_od_housing");

    close(out.stress_hoop_housing, 10475.764872909107, "stress_hoop_housing");
    close(out.housing_ms, 5.682089646840373, "housing_ms");
    close(out.stress_hoop_bushing, -31407.670580722985, "stress_hoop_bushing");
    close(out.bushing_ms, 0.5919677924375706, "bushing_ms");

    close(out.ed_actual, 1.5, "ed_actual");
    close(out.ed_min_sequence, 2.0748707835320768, "ed_min_sequence");
    close(out.ed_min_strength, 0.06482182611918386, "ed_min_strength");

    close(out.install_force, 1036.0361252090597, "install_force");
    close(out.retained_install_force, 1036.0361252090597, "retained_install_force");

    close(out.delta_total, 0.0014999999999999458, "delta_total");
    close(out.delta_thermal, 0.0, "delta_thermal");
    close(out.delta_user, 0.0014999999999999458, "delta_user");

    // The real, non-trivial part of this fixture: governing is edge-
    // distance sequencing, and it's a FAIL (negative margin) - proves the
    // port reproduces the failure direction, not just magnitudes.
    assert_eq!(out.governing.name, "Edge distance (sequencing)");
    close(out.governing.margin, -0.27706341430740444, "governing.margin");

    close(out.bore_tol.lower, 0.5, "bore_tol.lower");
    close(out.bore_tol.upper, 0.5, "bore_tol.upper");
    close(out.interference_tol.lower, 0.0015, "interference_tol.lower");
    close(out.od_tol.lower, 0.5015, "od_tol.lower");
    close(out.od_tol.upper, 0.5015, "od_tol.upper");
    close(out.achieved_interference_tol.nominal, 0.0014999999999999458, "achieved_interference_tol.nominal");
    assert_eq!(out.tolerance_status, bushing_solver::tolerance::ToleranceStatus::Ok);
}
