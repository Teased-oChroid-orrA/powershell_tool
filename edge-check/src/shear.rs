//! The shear-out plane taken from the elastic stress field instead of assumed.
//!
//! The handbook rule (`2 Fsu t (e - a cos 40 deg)`, MMPDS lug practice) fixes where the two shear-out planes leave the hole
//! (40 degrees from the load line) and runs them parallel to the load. Here a plane is a straight line from a point of the
//! bore wall to the free edge, described by two angles:
//!
//! * `start_deg` (phi): where it leaves the wall, measured at the hole centre from the load line toward the edge. The point
//!   is `(e - a cos phi, a sin phi)` with the free edge at `x = 0`;
//! * `incline_deg` (psi): its inclination to the load line, positive when it opens away from the load line toward the edge.
//!
//! The mean shear along it (the traction component along the plane, averaged over its length, Gauss quadrature over the
//! unit-case fields exactly as [`crate::field`] does) is a linear combination of the fit's unit field and the pin's, so
//! the load at which it reaches `Fsu` follows without a solve. [`scan`] evaluates a grid of planes, refines about the
//! lowest, and returns the plane of least capacity next to the two references: the handbook's own 40 degree planes and the
//! tangent planes `phi = 90` that `FieldResponse` uses.
//!
//! The planes are chosen from the field, not from a rule: `start_deg` and `incline_deg` have no preferred value. Elastic
//! mean shear is a criterion of full shear redistribution (as `Mode::ShearOut` is), so the plane found is where that
//! criterion is weakest, not a prediction of where a real plate tears. The FE shear path
//! (`lug_solver::shear_path`) is the elastic-plastic counterpart.

use crate::field::{gauss_legendre_unit, UnitFieldSource};
use crate::types::{Geometry, Stress, Strengths};
use std::f64::consts::PI;

/// One candidate plane and what it carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShearPlane {
    pub start_deg: f64,
    pub incline_deg: f64,
    /// Length from the wall to the free edge, in.
    pub length: f64,
    /// Distance of the plane's end on the free edge from the load line, in (`0` = the planes of both halves meet there).
    pub edge_y: f64,
    /// Pin load at which the mean shear on the plane reaches `Fsu` (fit pressure charged), lbf; `INFINITY` if it never does.
    pub capacity: f64,
}

/// The scan's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ShearScan {
    /// The plane of least capacity.
    pub best: ShearPlane,
    /// The handbook's start angle with the planes parallel to the load, evaluated on the field.
    pub classical: ShearPlane,
    /// The tangent planes `y = +-a` (`start_deg = 90`, `incline_deg = 0`): the existing `Mode::ShearOut`.
    pub tangent: ShearPlane,
    /// Planes evaluated.
    pub evaluated: usize,
}

/// Mean shear of the three unit cases along a plane.
#[derive(Debug, Clone, Copy)]
struct UnitShear {
    fit: f64,
    pin_half: f64,
    pin_full: f64,
    length: f64,
}

/// Start angles of the coarse grid, degrees (the plane must leave the wall, so 0 is excluded).
const START_GRID: [f64; 9] = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0];
/// Inclinations of the coarse grid, degrees.
const INCLINE_GRID: [f64; 11] = [-20.0, -15.0, -10.0, -5.0, 0.0, 5.0, 10.0, 15.0, 20.0, 30.0, 40.0];
/// Quadrature points along a plane (the integrand is smooth but steep near the bore).
const NG: usize = 24;
/// Smallest start angle and plane length a candidate may have (degenerate slivers carry no meaning).
const MIN_START_DEG: f64 = 5.0;

/// The geometry of a plane, or `None` if it is not one: it must leave the wall outward, stay on its side of the load
/// line and have length.
fn geometry(geom: &Geometry, start_deg: f64, incline_deg: f64) -> Option<(f64, f64, f64, f64)> {
    let (a, e) = (geom.bore_radius, geom.edge);
    let (phi, psi) = (start_deg.to_radians(), incline_deg.to_radians());
    if !(start_deg >= MIN_START_DEG && start_deg <= 90.0) || psi.abs() >= 0.5 * PI {
        return None;
    }
    // Leaves the wall outward: the direction (-cos psi, sin psi) against the outward normal (-cos phi, sin phi).
    if (phi - psi).cos() < 0.0 {
        return None;
    }
    let (x0, y0) = (e - a * phi.cos(), a * phi.sin());
    let length = x0 / psi.cos();
    // Does not cross the load line before the edge.
    if y0 + x0 * psi.tan() < 0.0 || x0 <= 0.0 {
        return None;
    }
    Some((x0, y0, psi, length))
}

fn unit_shear(src: &dyn UnitFieldSource, geom: &Geometry, start_deg: f64, incline_deg: f64) -> Option<UnitShear> {
    let (x0, y0, psi, length) = geometry(geom, start_deg, incline_deg)?;
    let (dx, dy) = (-psi.cos(), psi.sin());
    let mut acc = [0.0; 3];
    for (u, w) in gauss_legendre_unit(NG) {
        let s = u * length;
        let (px, py) = (x0 + dx * s, y0 + dy * s);
        let t: [Stress; 3] = src.stress_all(px, py)?;
        for (k, st) in t.iter().enumerate() {
            acc[k] += w * st.shear_along(dx, dy);
        }
    }
    Some(UnitShear { fit: acc[0], pin_half: acc[1], pin_full: acc[2], length })
}

/// Pin load at which `|fit_pressure * tau_fit + P * tau_pin| = fsu`, the smallest such `P >= 0`: with the fit holding the
/// back of the bore (pin load below `pi a t p`, the `FieldResponse` rule) the pin load is shared over the whole bore
/// (`pin_full`), above it only the loaded half carries it (`pin_half`).
fn capacity(u: &UnitShear, geom: &Geometry, fit_pressure: f64, fsu: f64) -> f64 {
    let base = fit_pressure * u.fit;
    if base.abs() >= fsu {
        return 0.0;
    }
    let switch = if fit_pressure > 0.0 { PI * geom.bore_radius * geom.thickness * fit_pressure } else { 0.0 };
    let mut best = f64::INFINITY;
    let mut consider = |tau_pin: f64, lo: f64, hi: f64| {
        if tau_pin == 0.0 {
            return;
        }
        for target in [fsu, -fsu] {
            let p = (target - base) / tau_pin;
            if p >= lo && p <= hi && p < best {
                best = p;
            }
        }
    };
    if switch > 0.0 {
        consider(u.pin_full, 0.0, switch);
    }
    consider(u.pin_half, switch, f64::INFINITY);
    best
}

fn plane(src: &dyn UnitFieldSource, geom: &Geometry, mat: &Strengths, fit_pressure: f64, start_deg: f64, incline_deg: f64) -> Option<ShearPlane> {
    let u = unit_shear(src, geom, start_deg, incline_deg)?;
    let (x0, y0, psi, _) = geometry(geom, start_deg, incline_deg)?;
    Some(ShearPlane { start_deg, incline_deg, length: u.length, edge_y: (y0 + x0 * psi.tan()).max(0.0), capacity: capacity(&u, geom, fit_pressure, mat.fsu) })
}

/// Scans the planes over `src` and returns the one of least capacity with the two reference planes.
///
/// A coarse grid over both angles finds the basin (the capacity surface has one broad minimum in the cases tried, but the
/// grid does not assume it); the best cell is then refined by halving the step about it, a few rounds, never leaving the
/// admissible region.
pub fn scan(src: &dyn UnitFieldSource, geom: &Geometry, mat: &Strengths, fit_pressure: f64) -> Result<ShearScan, String> {
    let miss = |what: &str| format!("the stress field has no value on the {what} shear plane");
    let classical = plane(src, geom, mat, fit_pressure, 40.0, 0.0).ok_or_else(|| miss("40 degree"))?;
    let tangent = plane(src, geom, mat, fit_pressure, 90.0, 0.0).ok_or_else(|| miss("tangent"))?;
    let mut evaluated = 2;
    let mut best: Option<ShearPlane> = None;
    let consider = |p: ShearPlane, best: &mut Option<ShearPlane>| {
        if best.is_none_or(|b| p.capacity < b.capacity) {
            *best = Some(p);
        }
    };
    for &phi in &START_GRID {
        for &psi in &INCLINE_GRID {
            if let Some(p) = plane(src, geom, mat, fit_pressure, phi, psi) {
                evaluated += 1;
                consider(p, &mut best);
            }
        }
    }
    let mut best = best.ok_or("no admissible shear plane")?;
    let (mut dphi, mut dpsi) = (5.0, 2.5);
    for _ in 0..4 {
        let centre = best;
        for i in -2..=2 {
            for j in -2..=2 {
                if i == 0 && j == 0 {
                    continue;
                }
                let (phi, psi) = (centre.start_deg + f64::from(i) * dphi, centre.incline_deg + f64::from(j) * dpsi);
                if let Some(p) = plane(src, geom, mat, fit_pressure, phi, psi) {
                    evaluated += 1;
                    if p.capacity < best.capacity {
                        best = p;
                    }
                }
            }
        }
        dphi *= 0.5;
        dpsi *= 0.5;
    }
    Ok(ShearScan { best, classical, tangent, evaluated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytic::AnalyticField;
    use crate::fem::MeshSpec;
    use crate::field::FieldResponse;
    use crate::kfem::KernelFem;
    use crate::model::Response;
    use crate::types::{Loads, Mode};
    use mechanics_core::materials::get_material;

    fn setup(ed: f64) -> (Geometry, Strengths) {
        let a = 0.125;
        let reach = (3.0 * ed * 2.0 * a).max(10.0 * a);
        (Geometry { bore_radius: a, edge: ed * 2.0 * a, thickness: 0.125, plate_far: reach, plate_half_height: reach, plane_angle_deg: 40.0 }, Strengths::from_material(get_material("al7075")))
    }

    /// Mean shear of planes parallel to the load that leave the wall at `phi` is the x-force of the pin's traction on the
    /// arc `|theta| < phi` over their area `2 t (e - a cos phi)` (equilibrium of the plug); a cosine bearing load carries
    /// `(2 phi + sin 2 phi) / pi` of the resultant there. The scan at `psi = 0` must reproduce that closed form.
    #[test]
    fn parallel_planes_reproduce_the_equilibrium_closed_form() {
        for ed in [1.5, 2.0, 3.0] {
            let (g, m) = setup(ed);
            let f = AnalyticField::build(g.bore_radius, g.edge, g.thickness, m.nu).unwrap();
            for phi_deg in [20.0_f64, 40.0, 60.0, 90.0] {
                let phi = phi_deg.to_radians();
                let share = (2.0 * phi + (2.0 * phi).sin()) / PI;
                let closed = 2.0 * m.fsu * g.thickness * (g.edge - g.bore_radius * phi.cos()) / share;
                let got = plane(&f, &g, &m, 0.0, phi_deg, 0.0).unwrap().capacity;
                assert!((got / closed - 1.0).abs() < 0.01, "e/D {ed} phi {phi_deg}: scan {got:.1} closed form {closed:.1}");
            }
        }
    }

    /// The tangent plane is the `Mode::ShearOut` of `FieldResponse`: at the capacity the scan reports, that response's
    /// shear-out margin is zero, with and without a fit pressure (which also exercises the pin-case switch).
    #[test]
    fn the_tangent_plane_is_the_existing_shear_out_mode() {
        for ed in [1.5, 2.0, 3.0] {
            let (g, m) = setup(ed);
            let f = AnalyticField::build(g.bore_radius, g.edge, g.thickness, m.nu).unwrap();
            let resp = FieldResponse::build(&f, &g, &m).unwrap();
            for p in [0.0, 2000.0, 8000.0] {
                let s = scan(&f, &g, &m, p).unwrap();
                let margin = resp.margins(&Loads { fit_pressure: p, pin_load: s.tangent.capacity }, 1.0).into_iter().find(|x| x.mode == Mode::ShearOut).unwrap().margin;
                assert!(margin.abs() < 1e-3, "e/D {ed} p {p}: shear-out margin {margin} at {:.1} lbf", s.tangent.capacity);
            }
        }
    }

    /// The best plane is never worse than the two references, the fit pressure lowers every capacity, and the optimum for
    /// the default geometry leaves the wall between 45 and 70 degrees (the handbook's 40 is not where the field is weakest).
    #[test]
    fn the_best_plane_beats_the_references_and_the_fit_costs_capacity() {
        for ed in [1.5, 2.0, 3.0] {
            let (g, m) = setup(ed);
            let f = AnalyticField::build(g.bore_radius, g.edge, g.thickness, m.nu).unwrap();
            let (s0, s1) = (scan(&f, &g, &m, 0.0).unwrap(), scan(&f, &g, &m, 5000.0).unwrap());
            for s in [&s0, &s1] {
                assert!(s.best.capacity <= s.classical.capacity && s.best.capacity <= s.tangent.capacity);
                assert!((45.0..=70.0).contains(&s.best.start_deg), "{:?}", s.best);
            }
            assert!(s1.best.capacity < s0.best.capacity);
        }
    }

    /// A plane that diverges toward the edge needs the plate to open, so the family's minimum is on the planes that
    /// converge until they meet on the load line at the free edge; the scan reports where it stopped (`edge_y`).
    #[test]
    fn the_minimum_is_the_converging_plane_that_just_reaches_the_load_line() {
        let (g, m) = setup(2.0);
        let f = AnalyticField::build(g.bore_radius, g.edge, g.thickness, m.nu).unwrap();
        let s = scan(&f, &g, &m, 0.0).unwrap();
        assert!(s.best.incline_deg < 0.0, "{:?}", s.best);
        assert!(s.best.edge_y < 0.05 * g.bore_radius, "{:?}", s.best);
    }

    /// The picked angles hold under mesh refinement of the FE the field can also come from, and so does the capacity.
    #[test]
    fn the_picked_plane_is_stable_under_mesh_refinement() {
        for ed in [1.5, 2.0, 3.0] {
            let (g, m) = setup(ed);
            let coarse = KernelFem::solve(&g, m.e, m.nu, MeshSpec::default()).unwrap();
            let fine = KernelFem::solve(&g, m.e, m.nu, MeshSpec { n_radial: 24, n_arc: [6, 9, 27], grade: 2.0 }).unwrap();
            let (a, b) = (scan(&coarse, &g, &m, 0.0).unwrap().best, scan(&fine, &g, &m, 0.0).unwrap().best);
            assert!((a.start_deg - b.start_deg).abs() <= 5.0, "e/D {ed}: coarse {a:?} fine {b:?}");
            assert!((a.incline_deg - b.incline_deg).abs() <= 2.5, "e/D {ed}: coarse {a:?} fine {b:?}");
            assert!((a.capacity / b.capacity - 1.0).abs() < 0.02, "e/D {ed}: coarse {a:?} fine {b:?}");
        }
    }

    /// Where the FE and the closed-form field disagree is the field's own error: the best capacity differs by under 5 %.
    #[test]
    fn the_analytic_and_fe_fields_pick_alike() {
        for ed in [1.5, 2.0, 3.0] {
            let (g, m) = setup(ed);
            let f = AnalyticField::build(g.bore_radius, g.edge, g.thickness, m.nu).unwrap();
            let fem = KernelFem::solve(&g, m.e, m.nu, MeshSpec::default()).unwrap();
            let (a, b) = (scan(&f, &g, &m, 0.0).unwrap().best, scan(&fem, &g, &m, 0.0).unwrap().best);
            assert!((a.capacity / b.capacity - 1.0).abs() < 0.05, "e/D {ed}: analytic {a:?} fem {b:?}");
        }
    }
}
