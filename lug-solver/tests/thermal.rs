//! Thermal fit change (upgrade plan item 9).

use lug_solver::*;

const AL: Material = Material { e_psi: 10.3e6, nu: 0.33 };
const STEEL: Material = Material { e_psi: 29e6, nu: 0.30 };

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

fn bushing(interference_dia: f64) -> BushingSpec {
    BushingSpec { inner_dia: 0.375, material: STEEL, interference_dia, friction: 0.2 }
}

// Per degree F (al 12.8e-6, steel 6.5e-6).
const AL_CTE: f64 = 12.8e-6;
const ST_CTE: f64 = 6.5e-6;

fn unloaded_fit(interference: f64, thermal: Thermal) -> f64 {
    let m = LugModel::build_bushed(&lug(), AL, MeshSpec::default(), true, PlaneMode::Stress, Some(bushing(interference))).unwrap();
    let pin = PinSpec { thermal, ..PinSpec::new(0.374, 0.0) };
    m.solve(pin, LoadCase { load_lbf: 0.0, angle_deg: 0.0 }).unwrap().bushing.unwrap().fit_pressure_unloaded
}

#[test]
fn the_thermal_helpers_follow_free_expansion() {
    let t = Thermal { delta_t: 100.0, alpha_lug: AL_CTE, alpha_bushing: ST_CTE, alpha_pin: ST_CTE };
    // A steel bushing in an aluminium hole loses interference when heated: 100 F x 0.5 in x 6.3e-6.
    assert!((t.interference(0.001, 0.5) - (0.001 - 100.0 * 0.5 * 6.3e-6)).abs() < 1e-12);
    // A steel pin in a steel bushing stays the same size relative to it.
    assert!((t.pin_diameter(0.374, true) - 0.374).abs() < 1e-12);
    // The same pin against an aluminium hole gets looser when heated.
    assert!(t.pin_diameter(0.374, false) < 0.374);
    assert_eq!(Thermal::default().interference(0.001, 0.5), 0.001);
}

#[test]
fn heating_a_steel_bushing_in_aluminium_relaxes_the_fit_linearly() {
    let thermal = |dt: f64| Thermal { delta_t: dt, alpha_lug: AL_CTE, alpha_bushing: ST_CTE, alpha_pin: ST_CTE };
    let p0 = unloaded_fit(0.001, Thermal::default());
    let (hot, cold) = (unloaded_fit(0.001, thermal(100.0)), unloaded_fit(0.001, thermal(-100.0)));
    eprintln!("THERMAL fit pressure: cold {cold:.0} nominal {p0:.0} hot {hot:.0}");
    assert!(hot < p0 && p0 < cold);
    // Linear elasticity: the pressure is proportional to the net interference.
    let d_eff = |dt: f64| 0.001 + dt * 0.5 * (ST_CTE - AL_CTE);
    for (dt, p) in [(100.0, hot), (-100.0, cold)] {
        let want = p0 * d_eff(dt) / 0.001;
        assert!((p / want - 1.0).abs() < 0.02, "dT {dt}: {p:.0} vs linear {want:.0}");
    }
    // Enough heating loses the fit altogether.
    let lost = unloaded_fit(0.001, thermal(400.0));
    assert!(lost < 0.05 * p0, "fit pressure {lost} after 400 F");
}

#[test]
fn a_thermal_change_equals_the_equivalent_explicit_fit() {
    let t = Thermal { delta_t: 150.0, alpha_lug: AL_CTE, alpha_bushing: ST_CTE, alpha_pin: ST_CTE };
    let a = unloaded_fit(0.001, t);
    let b = unloaded_fit(t.interference(0.001, 0.5), Thermal::default());
    assert!((a / b - 1.0).abs() < 1e-6, "{a} vs {b}");
}
