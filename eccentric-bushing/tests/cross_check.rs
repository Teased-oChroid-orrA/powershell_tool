//! Differential check against the independent bushing-and-housing contact FE of `edge-check` (own banded solver, own
//! contact formulation, NACA-validated): the same concentric plane-strain fit must give the same interface pressure,
//! and both must give the closed-form Lame pressure of their own housing.

use eccentric_bushing::{analyze, Elasticity, Inputs};
use edge_check::contact::ContactMesh;
use edge_check::fem::MeshSpec;
use edge_check::types::{BushingSpec, Geometry};

const A: f64 = 0.25; // bore radius
const RI: f64 = 0.19; // bushing bore radius
const DELTA: f64 = 0.001; // radial interference
const E_H: f64 = 10.4e6;
const NU_H: f64 = 0.33;
const E_B: f64 = 17.0e6;
const NU_B: f64 = 0.33;

/// Plane-strain Lame pressure of a bushing in a housing of outer radius `b`.
fn lame_plane_strain(b: f64) -> f64 {
    let h = (1.0 + NU_H) / E_H * A * ((1.0 - 2.0 * NU_H) * A * A + b * b) / (b * b - A * A);
    let bu = (1.0 + NU_B) / E_B * A * ((1.0 - 2.0 * NU_B) * A * A + RI * RI) / (A * A - RI * RI);
    DELTA / (h + bu)
}

#[test]
fn the_kernel_and_the_banded_contact_fe_agree_on_the_concentric_fit_pressure() {
    // edge-check: a plate large enough to be a half-space around the bore (Lame with b -> infinity).
    let geom = Geometry { bore_radius: A, edge: 30.0 * A, thickness: 0.5, plate_far: 30.0 * A * 3.0, plate_half_height: 30.0 * A * 3.0, plane_angle_deg: 40.0 };
    let spec = BushingSpec { inner_radius: RI, interference: DELTA, e: E_B, nu: NU_B, friction: 0.15 };
    let banded = ContactMesh::build(&geom, &spec, E_H, NU_H, MeshSpec { n_radial: 10, n_arc: [3, 4, 10], grade: 2.0 }, 3, true).unwrap();
    let (p_banded, _) = banded.fit_only(DELTA).unwrap();
    let infinite = lame_plane_strain(1e6);
    assert!((p_banded / infinite - 1.0).abs() < 0.01, "banded FE {p_banded} vs Lame {infinite}");

    // kernel: a boss of 40 bore diameters (Lame with b = 20 bore diameters of radius), plane strain, same materials.
    let boss_factor = 40.0;
    let i = Inputs {
        bore_dia: 2.0 * A,
        housing_od: boss_factor * 2.0 * A,
        edge_distance: None,
        bushing_id: 2.0 * RI,
        offset: 0.0,
        interference_dia: 2.0 * DELTA,
        thickness: 0.5,
        housing: Elasticity::iso(E_H, NU_H),
        bushing: Elasticity::iso(E_B, NU_B),
        friction: 0.15,
        pin: Elasticity::iso(29.0e6, 0.3),
        pin_friction: 0.1,
        pin_clearance_dia: 0.001,
        credit_pin_load: false,
        load_lbf: 0.0,
        load_angle_deg: 90.0,
        direct_onset: false,
        min_wall: 0.0,
        plane_strain: true,
        mesh_size: None,
    };
    let a = analyze(&i).unwrap();
    let want = lame_plane_strain(0.5 * i.housing_od);
    assert!((a.fit_pressure_mean / want - 1.0).abs() < 0.01, "kernel {} vs Lame {want}", a.fit_pressure_mean);
    // The two finite-element answers differ by no more than the housing-size effect between their Lame references.
    let expected_ratio = want / infinite;
    assert!((a.fit_pressure_mean / p_banded / expected_ratio - 1.0).abs() < 0.015, "kernel {} vs banded {p_banded} (Lame ratio {expected_ratio})", a.fit_pressure_mean);
}
