//! Differential test against real output from engineering.toolbox's own
//! TypeScript `computeBushing`, for the countersink/flanged geometry path
//! `differential.rs`'s base fixture never exercises (it's `bushingType:
//! 'straight'`, `idType: 'straight'`). This is the actual proof the
//! countersink corner-enumeration/neck-wall/bearing-profile port
//! (`countersink.rs`, `geometry.rs`, `bearing.rs`) is correct against the
//! real production engine, not just internally self-consistent.
//!
//! Golden numbers were captured by running the real TS engine on
//! 2026-08-29. Reproduce with:
//!
//! ```sh
//! cd ~/Claude/Projects/engineering.toolbox
//! npx tsx -e "
//!   import('./src/lib/core/bushing').then(({computeBushing}) => {
//!     console.log(JSON.stringify(computeBushing({
//!       units: 'imperial', boreDia: 0.5, idBushing: 0.375, interference: 0.0015,
//!       housingLen: 0.5, housingWidth: 1.5, edgeDist: 0.75,
//!       bushingType: 'countersink', idType: 'countersink',
//!       csMode: 'depth_angle', csDia: 0.5, csDepth: 0.08, csAngle: 100,
//!       csDiaTolPlus: 0.002, csDiaTolMinus: 0.0, csDepthTolPlus: 0.005, csDepthTolMinus: 0.0,
//!       extCsMode: 'depth_angle', extCsDia: 0.6, extCsDepth: 0.06, extCsAngle: 100,
//!       extCsDiaTolPlus: 0.002, extCsDiaTolMinus: 0.0, extCsDepthTolPlus: 0.005, extCsDepthTolMinus: 0.0,
//!       flangeOd: 0.75, flangeThk: 0.063,
//!       matHousing: 'al7075', matBushing: 'bronze', friction: 0.15, dT: 0,
//!       minWallStraight: 0.05, minWallNeck: 0.03
//!     }), null, 2));
//!   });
//! "
//! ```
//!
//! `bushingType: 'countersink'` combines an external OD countersink with a
//! `flangeOd`/`flangeThk` also set (harmlessly ignored by the TS source
//! when `bushingType !== 'flanged'`, same as this port's own
//! `geometry.rs`) - the useful part of this fixture is the internal ID
//! countersink actually thinning the wall (`neckWall` < `sleeveWall`,
//! `wallNeckNominal` != `wallNeck` - proving the corner worst-case search
//! moves the number, not just that geometry resolves without panicking).

use bushing_solver::countersink::CsMode;
use bushing_solver::geometry::{BushingType, IdType};
use bushing_solver::solve::{compute, BushingInputs, EndConstraint};

fn golden_input() -> BushingInputs {
    BushingInputs {
        bore_dia: 0.5,
        id_bushing: 0.375,
        interference: 0.0015,
        housing_len: 0.5,
        housing_width: 1.5,
        edge_dist: 0.75,
        mat_housing: "al7075".to_string(),
        mat_bushing: "bronze".to_string(),
        friction: Some(0.15),
        end_constraint: EndConstraint::Free,
        min_wall_straight: 0.05,
        min_wall_neck: 0.03,

        bushing_type: BushingType::Countersink,
        id_type: IdType::Countersink,
        flange_od: 0.75,
        flange_thk: 0.063,

        cs_mode: CsMode::DepthAngle,
        cs_dia: 0.5,
        cs_depth: 0.08,
        cs_angle: 100.0,
        cs_dia_tol_plus: 0.002,
        cs_depth_tol_plus: 0.005,

        ext_cs_mode: CsMode::DepthAngle,
        ext_cs_dia: 0.6,
        ext_cs_depth: 0.06,
        ext_cs_angle: 100.0,
        ext_cs_dia_tol_plus: 0.002,
        ext_cs_depth_tol_plus: 0.005,

        ..Default::default()
    }
}

fn close(actual: f64, expected: f64, label: &str) {
    let diff = (actual - expected).abs();
    let tol = expected.abs() * 1e-6 + 1e-9;
    assert!(diff <= tol, "{label}: expected {expected}, got {actual} (diff {diff}, tol {tol})");
}

#[test]
fn matches_real_ts_engine_output_for_a_countersink_and_flanged_fixture() {
    let out = compute(&golden_input());

    close(out.od_installed, 0.5015, "od_installed");
    close(out.wall_straight, 0.06324999999999997, "wall_straight");
    close(out.wall_neck_nominal, 0.03941492814811576, "wall_neck_nominal");
    close(out.wall_neck, 0.03345616018514469, "wall_neck");
    assert!(out.wall_neck < out.wall_neck_nominal, "the corner worst-case search must actually move the number, not just pass nominal through");
    assert!(out.wall_neck < out.wall_straight, "an internal countersink must thin the wall below the uniform straight-wall value");
    assert!(!out.fail_neck, "0.03345... is still above the 0.03 minimum");

    let cs_id = out.cs_solved_id.expect("id_type is Countersink");
    close(cs_id.dia, 0.5656805748150736, "cs_solved_id.dia");
    close(cs_id.depth, 0.08, "cs_solved_id.depth");
    close(cs_id.angle_deg, 100.0, "cs_solved_id.angle_deg");
    let cs_od = out.cs_solved_od.expect("bushing_type is Countersink");
    close(cs_od.dia, 0.6445104311113051, "cs_solved_od.dia");
    close(cs_od.depth, 0.06, "cs_solved_od.depth");
    close(cs_od.angle_deg, 100.0, "cs_solved_od.angle_deg");

    // Pressure/hoop stress depend only on bore/ID/materials, not OD/ID
    // countersink geometry - unchanged from the straight-bushing fixture's
    // own golden values (differential.rs), proving countersink geometry
    // doesn't leak into physics it shouldn't touch.
    close(out.pressure, 8794.147762602435, "pressure");
    close(out.stress_hoop_housing, 10475.764872909107, "stress_hoop_housing");
    close(out.stress_hoop_bushing, -31407.670580722985, "stress_hoop_bushing");
    close(out.housing_ms, 5.682089646840373, "housing_ms");
    close(out.bushing_ms, 0.5919677924375706, "bushing_ms");
    close(out.install_force, 1036.0361252090597, "install_force");
    close(out.retained_install_force, 1036.0361252090597, "retained_install_force");

    close(out.ed_actual, 1.5, "ed_actual");
    close(out.ed_min_sequence, 2.0748707835320768, "ed_min_sequence");
    // ed_min_strength DOES move vs. the straight fixture's 0.06482182611918386
    // - the whole point of this fixture: t_eff_seq now comes from the real
    // bearing-profile worst-external-corner search, not the housing_len
    // shortcut.
    close(out.ed_min_strength, 0.07080483464684201, "ed_min_strength");

    assert_eq!(out.governing.name, "Edge distance (sequencing)");
    close(out.governing.margin, -0.27706341430740444, "governing.margin");
    let neck_candidate = out.candidates.iter().find(|c| c.name == "Neck wall thickness").expect("Neck wall thickness candidate must be present");
    close(neck_candidate.margin, 0.11520533950482315, "neck wall thickness candidate margin");

    assert_eq!(out.tolerance_status, bushing_solver::tolerance::ToleranceStatus::Ok);

    // Countersink derived-dimension tolerance propagation
    // (`cs_dia_tolerance_from_base`/`cs_depth_tolerance_from_base`, wired
    // into `compute` for the first time - previously computed in
    // isolation by `countersink.rs`'s own unit tests but never actually
    // reaching `BushingOutput`). `cs_mode`/`ext_cs_mode` are both
    // `DepthAngle` in this fixture, so diameter is the derived dimension
    // for both internal and external - depth is not derived here, and
    // must just pass its own resolved input straight through.
    let internal_dia = out.cs_internal_dia_tol.expect("id_type is Countersink");
    close(internal_dia.lower, 0.5656805748150736, "cs_internal_dia_tol.lower");
    close(internal_dia.upper, 0.5775981107410157, "cs_internal_dia_tol.upper");
    let internal_depth = out.cs_internal_depth_tol.expect("id_type is Countersink");
    close(internal_depth.lower, 0.08, "cs_internal_depth_tol.lower");
    close(internal_depth.upper, 0.085, "cs_internal_depth_tol.upper");
    let external_dia = out.cs_external_dia_tol.expect("bushing_type is Countersink");
    close(external_dia.lower, 0.6445104311113051, "cs_external_dia_tol.lower");
    close(external_dia.upper, 0.6564279670372473, "cs_external_dia_tol.upper");
    let external_depth = out.cs_external_depth_tol.expect("bushing_type is Countersink");
    close(external_depth.lower, 0.06, "cs_external_depth_tol.lower");
    close(external_depth.upper, 0.065, "cs_external_depth_tol.upper");
}
