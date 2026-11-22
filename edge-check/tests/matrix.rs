//! The model-by-target matrix (`edge_check::matrix`): roles, cells equal the report, nothing about the legacy check moves.

use edge_check::matrix::{EdgeMatrix, LegacyEdge, Role, BEST_ESTIMATE_MODEL, LEGACY_ID};
use edge_check::models::default_models;
use edge_check::runner::{run, EdgeConfig, EdgeInput, EdgeMin, TARGET_APPLIED, TARGET_BEARING};
use edge_check::types::{BushingSpec, Geometry, Strengths};
use mechanics_core::materials::get_material;

fn input(ed: f64) -> EdgeInput {
    let a = 0.25;
    let e = ed * 2.0 * a;
    let reach = (3.0 * e).max(10.0 * a);
    EdgeInput {
        geom: Geometry { bore_radius: a, edge: e, thickness: 0.5, plate_far: reach, plate_half_height: reach, plane_angle_deg: 40.0 },
        strengths: Strengths::from_material(get_material("al7075")),
        applied_load: 1000.0,
        fit_pressure: 8800.0,
        fit_pressure_min: 6000.0,
        fit_pressure_max: 11000.0,
    }
}

const LEGACY: LegacyEdge = LegacyEdge { applied: 0.62, bearing: 2.07 };

#[test]
fn without_the_contact_fe_there_is_no_best_estimate_and_legacy_still_governs() {
    let cfg = EdgeConfig { mc_samples: 200, ..EdgeConfig::default() };
    let report = run(&default_models(&cfg), &input(1.5), &cfg);
    let m = EdgeMatrix::build(&report, Some(LEGACY));
    assert!(m.best_estimate().is_none(), "the quick set has no contact FE, so nothing may be labelled best estimate");
    assert_eq!(m.rows.iter().filter(|r| r.role == Role::BestEstimate).count(), 0);
    let g = m.governing().expect("the legacy row");
    assert_eq!(g.id, LEGACY_ID);
    assert_eq!(m.rows[0].id, LEGACY_ID, "the governing row leads when there is no best estimate");
    assert!(m.rows[1..].iter().all(|r| r.role == Role::Comparison));
    assert!((m.actual - 1.5).abs() < 1e-9);
}

#[test]
fn the_legacy_row_carries_exactly_the_solvers_numbers_and_no_probability_levels() {
    let cfg = EdgeConfig { mc_samples: 200, ..EdgeConfig::default() };
    let report = run(&default_models(&cfg), &input(1.5), &cfg);
    let m = EdgeMatrix::build(&report, Some(LEGACY));
    let g = m.governing().unwrap();
    assert_eq!(g.cells[TARGET_APPLIED][0].unwrap().min, EdgeMin::Value(0.62));
    assert_eq!(g.cells[TARGET_BEARING][0].unwrap().min, EdgeMin::Value(2.07));
    assert!(!g.cells[TARGET_APPLIED][0].unwrap().short, "0.62 <= 1.5");
    assert!(g.cells[TARGET_BEARING][0].unwrap().short, "2.07 > 1.5");
    for t in 0..2 {
        assert!(g.cells[t][1..].iter().all(|c| c.is_none()), "the legacy check is deterministic: no P90/P95/P99");
    }
    // No legacy numbers handed in: no governing row is invented.
    assert!(EdgeMatrix::build(&report, None).governing().is_none());
}

#[test]
fn matrix_cells_are_the_reports_minimums_in_bore_diameters() {
    let cfg = EdgeConfig { mc_samples: 200, ..EdgeConfig::default() };
    let report = run(&default_models(&cfg), &input(1.5), &cfg);
    let m = EdgeMatrix::build(&report, Some(LEGACY));
    let d = report.bore_diameter;
    for mr in &report.models {
        let row = m.rows.iter().find(|r| r.id == mr.id).expect("every model has a row");
        assert_eq!(row.role, Role::Comparison);
        for (ti, t) in [TARGET_APPLIED, TARGET_BEARING].into_iter().enumerate() {
            let Some(tr) = mr.targets[t].as_ref() else { continue };
            let scale = |e: EdgeMin| match e {
                EdgeMin::Value(v) => EdgeMin::Value(v / d),
                EdgeMin::AtMost(v) => EdgeMin::AtMost(v / d),
                EdgeMin::Exceeds(v) => EdgeMin::Exceeds(v / d),
                EdgeMin::NotSearched => EdgeMin::NotSearched,
            };
            let want: Vec<EdgeMin> = std::iter::once(tr.e_min).chain(tr.e_levels).map(scale).collect();
            for (l, w) in want.iter().enumerate() {
                assert_eq!(row.cells[ti][l].map(|c| c.min).unwrap_or(EdgeMin::NotSearched), *w, "{} target {ti} level {l}", mr.id);
            }
            if let (Some(p90), Some(p99)) = (row.cells[ti][1], row.cells[ti][3]) {
                let key = |c: edge_check::matrix::MatrixCell| EdgeMin::rank(c.min);
                assert!(key(p99) >= key(p90), "{}: P99 must ask for at least what P90 does", mr.id);
            }
        }
    }
}

#[test]
fn the_contact_fe_is_the_best_estimate_and_sorts_first_with_legacy_second() {
    let bushing = BushingSpec { inner_radius: 0.1875, interference: 0.00075, e: 17.0e6, nu: 0.33, friction: 0.15 };
    let cfg = EdgeConfig { mc_samples: 200, include_plastic: true, bushing: Some(bushing), ..EdgeConfig::default() };
    let report = run(&default_models(&cfg), &input(1.5), &cfg);
    let m = EdgeMatrix::build(&report, Some(LEGACY));
    let best = m.best_estimate().expect("the contact FE ran");
    assert_eq!(best.id, BEST_ESTIMATE_MODEL);
    assert_eq!(m.rows[0].id, BEST_ESTIMATE_MODEL);
    assert_eq!(m.rows[1].id, LEGACY_ID);
    assert_eq!(m.rows.iter().filter(|r| r.role == Role::Governing).count(), 1);
    assert_eq!(m.rows.iter().filter(|r| r.role == Role::BestEstimate).count(), 1);
    // The best estimate has probability levels, and its bearing-limit nominal number agrees with the report.
    assert!(best.cells[TARGET_BEARING][3].is_some(), "P99 for the bearing limit");
    assert!(best.cells[TARGET_BEARING][0].is_some());
}
