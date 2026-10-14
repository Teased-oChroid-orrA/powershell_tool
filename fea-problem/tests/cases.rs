//! Load cases: a case solves its own supports and loads at its own factors; no cases solve everything as entered.
use fea_problem::{solve, templates, Load, LoadCase, Problem};

fn plate() -> Problem {
    let mut p = templates::templates().remove(0).1;
    p.mesh.size = 1.0;
    p.mesh.adapt_passes = 0;
    p.loads.push(Load::Pressure { edge: "top".into(), p: 500.0 });
    p
}

#[test]
fn no_cases_means_everything_acts_and_a_problem_without_cases_saves_as_before() {
    let p = plate();
    assert_eq!(p.resolved(), p);
    let json = p.to_json().unwrap();
    assert!(!json.contains("cases") && !json.contains("active_case"), "an old file stays byte-compatible");
    assert_eq!(Problem::from_json(&json).unwrap(), p);
}

#[test]
fn a_case_picks_loads_and_scales_them_and_the_response_scales_with_the_factor() {
    let mut p = plate();
    // Case 1: the traction only. Case 2: the same traction at twice the factor. Case 3: the pressure only.
    p.add_case("Double pull");
    p.add_case("Pressure");
    p.cases[0].set_factor(1, 0.0);
    p.cases[1].set_factor(0, 2.0);
    p.cases[1].set_factor(1, 0.0);
    p.cases[2].set_factor(0, 0.0);
    let run = |p: &Problem, k: usize| {
        let mut q = p.clone();
        q.active_case = k;
        solve(&q, None).unwrap()
    };
    let (one, two, press) = (run(&p, 0), run(&p, 1), run(&p, 2));
    let ratio = two.summary.max_displacement.value / one.summary.max_displacement.value;
    assert!((ratio - 2.0).abs() < 1e-9, "linear elasticity: twice the load, twice the displacement ({ratio})");
    assert!((two.summary.applied[0] / one.summary.applied[0] - 2.0).abs() < 1e-9);
    assert!(press.summary.applied[0].abs() < 1e-6 && press.summary.applied[1].abs() > 1.0, "only the pressure acts in case 3: {:?}", press.summary.applied);
    assert!(one.summary.applied[1].abs() < 1e-6, "no pressure in case 1");
    // The result stays tied to the full problem, so stale checks and saved files see the cases.
    assert_eq!(one.problem, { let mut q = p.clone(); q.active_case = 0; q });
}

#[test]
fn a_support_switched_off_in_a_case_leaves_the_model_free() {
    let mut p = plate();
    p.add_case("Free");
    p.cases[1].set_support(0, false);
    p.cases[1].set_support(1, false);
    assert!(solve(&p, None).is_err(), "no support at all is not a solvable static problem");
    p.active_case = 0;
    assert!(solve(&p, None).is_ok());
}

#[test]
fn removing_a_support_or_load_keeps_the_cases_pointing_at_the_same_entries() {
    let mut p = plate();
    p.loads.push(Load::Force { edge: "bottom".into(), fx: 1.0, fy: 0.0, fz: 0.0 });
    p.add_case("B");
    p.cases[1].set_factor(2, 3.0);
    p.cases[1].set_factor(0, 0.5);
    p.cases[1].set_support(1, false);
    p.remove_load(0);
    assert_eq!(p.cases[1].load_factors, vec![(1, 3.0)], "the removed load's factor is gone and the later one moved up");
    assert_eq!(p.loads.len(), 2);
    p.remove_support(0);
    assert_eq!(p.cases[1].supports_off, vec![0], "support 1 became support 0");
    // One case left is no cases.
    p.remove_case(1);
    assert!(p.cases.is_empty() && p.active_case == 0 && p.resolved() == p);
}

#[test]
fn cases_round_trip_through_json() {
    let mut p = plate();
    p.add_case("Second");
    p.cases[1].set_factor(0, 0.25);
    let q = Problem::from_json(&p.to_json().unwrap()).unwrap();
    assert_eq!(q.cases, p.cases);
    assert_eq!(q.active_case, 1);
    let _: LoadCase = q.cases[0].clone();
}
