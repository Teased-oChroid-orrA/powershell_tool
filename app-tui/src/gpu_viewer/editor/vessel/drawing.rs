//! The vessel drawn to scale with its stress through the wall: a three-quarter section of the cylinder (a quarter cut
//! away, `editor/drawing.rs`), coloured by the thick-wall (Lamé) stress at each point's radius. Every value comes from
//! `pressure_vessel_solver::stress::stress_at_radius_with_thermal`, the same function the checks evaluate; nothing here
//! is a stress formula. The drawn length is only for the picture (the analysis is a long cylinder, per unit length).
use crate::gpu_viewer::{
    editor::drawing::{self, Mesh},
    scene::ScalarField,
    Scene,
};
use crate::toolboxes::pressure_vessel::model::PressureVesselModel;
use pressure_vessel_solver::failure::{tresca_stress, von_mises_stress};
use pressure_vessel_solver::stress::{stress_at_radius_with_thermal, StressState};

/// The drawn length, in outer diameters.
pub const LENGTH_PER_OD: f64 = 1.5;
/// Points across the wall on the end faces and the cut faces, so the colour follows the stress through the wall.
const ACROSS: usize = 24;

/// The fields the picture carries; the first is the one it opens on.
pub const FIELDS: [&str; 5] = ["Hoop stress", "Radial stress", "Axial stress", "von Mises stress", "Tresca stress"];

fn field_values(s: &StressState) -> [f64; 5] {
    [s.hoop, s.radial, s.axial, von_mises_stress(s), tresca_stress(s)]
}

/// The picture of `m` (drawn in inches, coloured in psi), or why there is none (inputs that make no vessel).
pub fn scene(m: &PressureVesselModel, name: &str) -> Result<Scene, String> {
    let geometry = m.geometry.map_err(|e| format!("the geometry is invalid ({e:?})"))?;
    let pressure = m.pressure.map_err(|e| format!("the pressure is invalid ({e:?})"))?;
    let thermal = m.thermal_loading(m.material());
    let (a, b) = (geometry.inner_radius, geometry.outer_radius);
    let length = LENGTH_PER_OD * 2.0 * b;
    let across = |z: f64| (0..=ACROSS).map(|k| (a + (b - a) * k as f64 / ACROSS as f64, z)).collect::<Vec<_>>();
    let mut mesh = Mesh::default();
    let none = |_: &[f64; 3]| 0.0;
    mesh.revolve(&[(a, length), (a, 0.0)], none);
    mesh.revolve(&[(b, 0.0), (b, length)], none);
    mesh.revolve(&across(length), none);
    mesh.revolve(&across(0.0), none);
    mesh.cut(&[across(length), across(0.0)], none);
    // Each vertex's stress at its own radius (the axis is y), clamped into the wall against rounding.
    let states: Vec<StressState> = mesh.positions.iter().map(|p| stress_at_radius_with_thermal(&geometry, &pressure, thermal.as_ref(), p[0].hypot(p[2]).clamp(a, b))).collect();
    let mut columns: [Vec<f64>; 5] = Default::default();
    for s in &states {
        for (column, v) in columns.iter_mut().zip(field_values(s)) {
            column.push(v);
        }
    }
    let primary = ScalarField::from_f64(FIELDS[0], &columns[0])?;
    mesh.values = primary.values;
    let fields = FIELDS[1..].iter().zip(&columns[1..]).map(|(label, v)| ScalarField::from_f64(label, v)).collect::<Result<Vec<_>, _>>()?;
    mesh.into_scene(format!("{name} | {}", m.material().name), FIELDS[0], primary.range, fields)
}

/// A scene with nothing visible in it, for inputs that make no vessel.
pub fn empty(name: &str) -> Scene {
    drawing::empty(format!("{name} | nothing to draw"), [0.0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolboxes::pressure_vessel::model::NumberTarget;

    fn extremes(v: &[f32]) -> (f32, f32) {
        (v.iter().copied().fold(f32::INFINITY, f32::min), v.iter().copied().fold(f32::NEG_INFINITY, f32::max))
    }

    #[test]
    fn the_contour_is_the_lame_stress_through_the_wall_of_the_shigley_case() {
        // OD 6, wall 1 (a = 2, b = 3), 5000 psi internal, closed ends: hoop 13000 psi at the bore, 8000 at the outside,
        // radial -5000 to 0, axial C1 = 4000 everywhere.
        let m = PressureVesselModel::default();
        let s = scene(&m, "case").unwrap();
        let hoop: Vec<f32> = s.samples.iter().map(|v| v[3]).collect();
        let (lo, hi) = extremes(&hoop);
        assert!((hi - 13000.0).abs() < 0.5 && (lo - 8000.0).abs() < 0.5, "{lo} {hi}");
        assert_eq!(s.range, [lo, hi]);
        assert_eq!(s.field_label, "Hoop stress");
        let field = |label: &str| s.fields.iter().find(|f| f.label == label).unwrap();
        assert_eq!(s.fields.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), FIELDS[1..]);
        let (rlo, rhi) = extremes(&field("Radial stress").values);
        assert!((rlo + 5000.0).abs() < 0.5 && rhi.abs() < 0.5, "{rlo} {rhi}");
        let (alo, ahi) = extremes(&field("Axial stress").values);
        assert!((alo - 4000.0).abs() < 0.5 && (ahi - 4000.0).abs() < 0.5, "{alo} {ahi}");
        // The bore is the hottest point of every check (thick wall, internal pressure).
        let (_, vm) = extremes(&field("von Mises stress").values);
        let bore = von_mises_stress(&pressure_vessel_solver::stress::stress_at_inner_surface(&m.geometry.unwrap(), &m.pressure.unwrap()));
        assert!((vm as f64 - bore).abs() < 0.5, "{vm} {bore}");
        // Drawn to scale: the length (1.5 OD = 9 in) is the unit span, the outer diameter 6 / 9 of it.
        let ext = |a: usize| s.positions.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max) - s.positions.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min);
        assert!((ext(1) - 1.0).abs() < 1e-6 && (ext(0) - 6.0 / 9.0).abs() < 1e-6, "{} {}", ext(0), ext(1));
    }

    #[test]
    fn a_temperature_differential_changes_the_picture_and_inputs_that_make_no_vessel_draw_nothing() {
        let mut m = PressureVesselModel::default();
        let before = scene(&m, "case").unwrap().samples;
        m.commit_number(NumberTarget::TemperatureDifferential, 300.0);
        assert_ne!(scene(&m, "case").unwrap().samples, before, "thermal stress is superposed on the hoop stress");
        m.commit_number(NumberTarget::WallThickness, 10.0);
        assert!(scene(&m, "case").unwrap_err().contains("geometry is invalid"));
        assert_eq!(empty("case").positions.len(), 3);
    }
}
