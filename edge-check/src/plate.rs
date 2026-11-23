//! A plate (a finite strip) with a hole near a free edge under a plate load: the plane-stress FE on `fea-core`, two
//! criteria, each its own column of the matrix:
//!
//! * **Net section** - the mean stress along the strip across the ligament between the hole and the nearer free edge
//!   (`y = 0`, `x` from the edge to the bore) against `Ftu`. For a centred hole under an axial load this equals
//!   `P / ((W - D) t)` exactly (equilibrium; tested). For a hole nearer one edge the ligament carries more than the
//!   section mean, and that is the number this criterion reports.
//! * **First yield** - the peak elastic von Mises stress on the bore and the near free edge against `Sy`.
//!
//! **Load and its reactions (stated, because a width and an angle alone do not define them).** The strip runs along `y`
//! between two free edges (`x = 0` and `x = W`); the hole is at `(e, 0)`. A load `F` at `angle` to the strip axis acts on
//! the strip's end at `y = +arm` (the distance from the hole centre to the loaded end):
//!
//! * its axial part `N = F cos(angle)` is a uniform traction over the end;
//! * its transverse part `V = F sin(angle)` is a parabolic shear traction over the end (zero at the free edges);
//! * the far end, at `y = -arm`, reacts both: a uniform axial traction `-N`, a parabolic shear `-V`, and the end moment
//!   `V * 2 arm` that balances the couple of the two shears, as a linear bending traction. The whole strip is therefore
//!   in equilibrium and the section through the hole carries `N`, the shear `V` and the moment `V * arm`.
//!
//! The direction of the transverse part (toward the near free edge or away from it) is not given, so both are computed and
//! the worse is reported. At 0 degrees only the axial part acts, and the strip is solved on its symmetric half (a uniform
//! end traction on a strip with free edges is the exact uniform stress state, so the strip needs no length for end effects).
//! The load must act at least one plate width from the hole: nearer, how the load is introduced would decide the result.
//!
//! The other models cannot represent a finite strip at all and are shown as unavailable with the reason.
//!
//! Not validated against test data: limiting cases only (the open-hole stress concentration, the published finite-width
//! one, the equilibrium of the net section, the beam stresses of a tiny hole under the shear and moment). Elastic, small
//! strain, plane stress; no hardening, no fracture, no bearing.

use crate::fem::{plate_mesh, MeshSpec, PlateMesh};
use crate::field::gauss_legendre_unit;
use crate::kfem::PERM;
use crate::matrix::{EdgeMatrix, MatrixCell, MatrixRow, Role};
use crate::runner::EdgeMin;
use crate::types::{Geometry, Strengths, Stress};
use fea_core::loads::{FaceField, Loads, SurfaceLoad};
use fea_core::locate::Locator;
use fea_core::{Dirichlet, Elastic, ElementKind, Mesh, Model, Physics};
use std::f64::consts::PI;

/// Column labels of the plate matrix (instead of the pin case's applied load / bearing limit).
pub const CRITERIA: [&str; 2] = ["Net section vs Ftu", "First yield at the hole"];

pub const WHY_SUPERPOSITION: &str = "the half-plane stress superposition has one free edge and no far field: it cannot represent a finite strip";
pub const WHY_ALLOWABLES: &str = "the tabulated allowables are pin-bearing and shear-out rules: they say nothing about a plate load";
pub const WHY_CONTACT: &str = "the contact FE is built around a pin loading the bore: with no pin it is a different, unvalidated use";

#[derive(Debug, Clone, PartialEq)]
pub struct PlateInput {
    pub bore_radius: f64,
    /// Bore centre to the nearer free edge; at most half the width.
    pub edge: f64,
    /// Strip width, edge to edge.
    pub width: f64,
    pub thickness: f64,
    pub material: Strengths,
    /// Plate load `F`, lbf.
    pub load: f64,
    /// Load direction against the strip axis, degrees, 0 to 90: 0 is along the strip, 90 is all shear.
    pub angle_deg: f64,
    /// Distance from the hole centre to the loaded end (and to the reacting far end), at least the width. Only a transverse
    /// part of the load uses it (the moment at the hole is `V * arm`).
    pub arm: f64,
}

/// The numbers behind the FE cells.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateNumbers {
    /// Load `F` the plate carries before the criterion fails at the actual geometry (lbf), worse transverse direction.
    pub net_section_capacity: f64,
    pub first_yield_capacity: f64,
    /// Peak von Mises on the bore and the free edge per unit gross stress `F / (W t)`.
    pub kt_gross: f64,
    /// Mean ligament stress per unit gross stress.
    pub ligament_over_gross: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlateEstimate {
    /// The matrix with the plane-stress FE as the only available row.
    pub matrix: EdgeMatrix,
    pub numbers: Option<PlateNumbers>,
}

/// The unit solutions: stress per lbf of axial load, and (full model only) per lbf of transverse load with its moment.
struct Fields {
    model: Model,
    u: Vec<Vec<f64>>,
    locator: Locator,
    /// Both halves are modelled (the shear case is not symmetric about `y = 0`).
    full: bool,
}

impl Fields {
    fn at(&self, case: usize, x: f64, y: f64) -> Option<Stress> {
        let (y, flip) = if !self.full && y < 0.0 { (-y, -1.0) } else { (y, 1.0) };
        let s = self.locator.stress_at(&self.model, &self.u[case], [x, y, 0.0], 0.0)?;
        Some(Stress { xx: s[0], yy: s[1], xy: s[3] * flip })
    }
    /// `ca` of the axial unit case plus `cv` of the transverse one.
    fn combined(&self, ca: f64, cv: f64, x: f64, y: f64) -> Option<Stress> {
        let a = self.at(0, x, y)?.scaled(ca);
        Some(if cv != 0.0 && self.u.len() > 1 { a.plus(self.at(1, x, y)?.scaled(cv)) } else { a })
    }
}

/// `full`: also the mirrored half and the transverse unit case, with the loaded end `arm` from the hole.
fn solve(a: f64, e: f64, w: f64, t: f64, mat: &Strengths, spec: MeshSpec, full: bool, arm: f64) -> Result<Fields, String> {
    // Axial only: the half length only has to leave the mesh well shaped. With a shear: the loaded end.
    let h = if full { arm } else { (w - e).max(8.0 * a) };
    let geom = Geometry { bore_radius: a, edge: e, thickness: t, plate_far: w - e, plate_half_height: h, plane_angle_deg: 40.0 };
    let PlateMesh { nodes, elems, na, n_theta_el } = plate_mesh(&geom, spec);
    // The strip's end (y = h): the outer ring edges lying on it, body on the left walking up the ring.
    let top: Vec<Vec<usize>> = (0..n_theta_el)
        .map(|j| vec![(2 * j) * na + na - 1, (2 * j + 2) * na + na - 1, (2 * j + 1) * na + na - 1])
        .filter(|f| f.iter().all(|&n| (nodes[n][1] - h).abs() < 1e-9 * h))
        .collect();
    if top.is_empty() {
        return Err("the strip's end face was not found in the mesh".into());
    }
    let mut mesh = Mesh::new(Physics::PlaneStress { thickness: t });
    let mut pos: Vec<[f64; 2]> = nodes.clone();
    // The mirrored half: nodes on y = 0 are shared, element rows keep their orientation by reversing the columns.
    let mut map: Vec<usize> = (0..nodes.len()).collect();
    if full {
        for (i, p) in nodes.iter().enumerate() {
            if p[1] != 0.0 {
                map[i] = pos.len();
                pos.push([p[0], -p[1]]);
            }
        }
    }
    for p in &pos {
        mesh.add_node([p[0], p[1], 0.0]);
    }
    let mut conn: Vec<usize> = elems.iter().flat_map(|c| PERM.iter().map(move |&k| c[k])).collect();
    if full {
        for c in &elems {
            let mirrored: Vec<usize> = (0..9).map(|k| map[c[3 * (k / 3) + (2 - k % 3)]]).collect();
            conn.extend(PERM.iter().map(|&k| mirrored[k]));
        }
    }
    mesh.add_block(ElementKind::Quad9, conn, Elastic::new(mat.e, mat.nu), "plate")?;

    // 1 lbf: forces per area; the kernel multiplies by the thickness.
    let sigma = 1.0 / (t * w);
    let axial_top: Vec<(Vec<usize>, SurfaceLoad)> = top.iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, sigma, 0.0]))).collect();
    let mut cases = vec![Loads { faces: axial_top, ..Loads::default() }];
    if full {
        // The reacting end: the mirrored faces, walked the other way to keep the body on the left.
        let bottom: Vec<Vec<usize>> = top.iter().map(|f| vec![map[f[1]], map[f[0]], map[f[2]]]).collect();
        cases[0].faces.extend(bottom.iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, -sigma, 0.0]))));
        // Transverse 1 lbf V at the top (parabolic over the width), its reaction -V at the bottom, and the bottom end
        // moment 2 arm V as a linear bending traction.
        let tau = move |x: f64| 6.0 * x * (w - x) / (w * w * w * t);
        let bend = move |x: f64| 12.0 * (2.0 * arm) * (x - 0.5 * w) / (t * w * w * w);
        let fx = |sign: f64| FaceField::new(move |p: &[f64; 3], _n: &[f64; 3]| [sign * tau(p[0]), 0.0, 0.0]);
        let (up, down) = (fx(1.0), fx(-1.0));
        let moment = FaceField::new(move |p: &[f64; 3], _n: &[f64; 3]| [0.0, bend(p[0]), 0.0]);
        let mut shear = Loads::default();
        shear.field_faces.extend(top.iter().map(|f| (f.clone(), up.clone())));
        shear.field_faces.extend(bottom.iter().map(|f| (f.clone(), down.clone())));
        shear.field_faces.extend(bottom.iter().map(|f| (f.clone(), moment.clone())));
        cases.push(shear);
    }
    let model = Model::new(mesh)?;
    let mut bc: Dirichlet = model.dirichlet();
    if full {
        // Statically determinate supports of a self-equilibrated load: the free edge's node on y = 0 fixed both ways, the
        // far face's node on y = 0 fixed along the strip. Their reactions are zero.
        let near = pos.iter().position(|p| p[0].abs() < 1e-12 && p[1] == 0.0).ok_or("no node on the free edge at y = 0")?;
        bc.fix(near, 0, 0.0);
        bc.fix(near, 1, 0.0);
        bc.fix(na - 1, 1, 0.0);
    } else {
        // Symmetry about the section through the bore (u_y = 0 on y = 0) and one u_x anchor on it at the far face.
        for (n, p) in pos.iter().enumerate() {
            if p[1] == 0.0 {
                bc.fix(n, 1, 0.0);
            }
        }
        bc.fix(na - 1, 0, 0.0);
    }
    let u: Vec<Vec<f64>> = model.solve_static_many(&cases, &bc)?.into_iter().map(|s| s.u).collect();
    let locator = model.locator();
    Ok(Fields { model, u, locator, full })
}

/// `(mean ligament stress, peak von Mises)` per lbf of load for the load mix `(ca, cv)`.
fn measures(f: &Fields, a: f64, e: f64, ca: f64, cv: f64) -> Result<(f64, f64), String> {
    let miss = |what: &str, x: f64, y: f64| format!("stress field has no value at the {what} probe ({x:.4}, {y:.4})");
    let mut ligament = 0.0;
    for (u, w) in gauss_legendre_unit(24) {
        let x = (e - a) * u;
        ligament += w * f.combined(ca, cv, x, 0.0).ok_or_else(|| miss("ligament", x, 0.0))?.yy;
    }
    let mut peak: f64 = 0.0;
    const NB: usize = 73;
    let sweep = if f.full { 2.0 * PI } else { PI };
    let nb = if f.full { 2 * (NB - 1) } else { NB - 1 };
    for k in 0..=nb {
        let phi = sweep * k as f64 / nb as f64;
        let (x, y) = (e + a * phi.cos(), a * phi.sin());
        peak = peak.max(f.combined(ca, cv, x, y).ok_or_else(|| miss("bore", x, y))?.von_mises());
    }
    const NE: usize = 25;
    for k in 0..NE {
        let y = 4.0 * a * ((k as f64) / (NE - 1) as f64).powi(2);
        for y in if f.full && y > 0.0 { vec![y, -y] } else { vec![y] } {
            peak = peak.max(f.combined(ca, cv, 0.0, y).ok_or_else(|| miss("free-edge", 0.0, y))?.von_mises());
        }
    }
    Ok((ligament, peak))
}

/// Why the input cannot be estimated, if it cannot.
pub fn validate(i: &PlateInput) -> Result<(), String> {
    let (a, e, w, t) = (i.bore_radius, i.edge, i.width, i.thickness);
    if !(a > 0.0 && a.is_finite() && t > 0.0 && t.is_finite() && w.is_finite() && e.is_finite()) {
        return Err("bore diameter, plate width and thickness must be finite and positive".into());
    }
    if !(i.load.is_finite() && i.load >= 0.0) {
        return Err("the load must be finite and not negative".into());
    }
    if !(i.angle_deg.is_finite() && (0.0..=90.0).contains(&i.angle_deg)) {
        return Err("the load angle must be between 0 (along the strip) and 90 degrees (all shear)".into());
    }
    if e <= a {
        return Err("edge distance must exceed the bore radius".into());
    }
    if 2.0 * e > w * (1.0 + 1e-12) {
        return Err("edge distance must not exceed half the plate width: the edge named is the nearer one".into());
    }
    if w - e <= a {
        return Err("the plate is narrower than the hole".into());
    }
    if i.angle_deg > 1e-9 && !(i.arm.is_finite() && i.arm >= w * (1.0 - 1e-12)) {
        return Err("the load must act at least one plate width from the hole (the arm), else how it is introduced decides the result".into());
    }
    Ok(())
}

/// `(mean ligament stress, peak von Mises)` per lbf at edge distance `e`, the worse transverse direction.
fn per_lbf(i: &PlateInput, e: f64, spec: MeshSpec) -> Result<(f64, f64), String> {
    let (a, w, t) = (i.bore_radius, i.width, i.thickness);
    let ang = i.angle_deg.to_radians();
    if i.angle_deg.abs() < 1e-9 {
        let f = solve(a, e, w, t, &i.material, spec, false, 0.0)?;
        return measures(&f, a, e, 1.0, 0.0);
    }
    let f = solve(a, e, w, t, &i.material, spec, true, i.arm)?;
    let (plus, minus) = (measures(&f, a, e, ang.cos(), ang.sin())?, measures(&f, a, e, ang.cos(), -ang.sin())?);
    Ok((plus.0.max(minus.0), plus.1.max(minus.1)))
}

/// The smallest edge distance (in) at which the criterion is met at `load`, between e/D 0.75 and half the width. The
/// margin rises with the edge distance on that range.
fn min_edge(i: &PlateInput, spec: MeshSpec, capacity: impl Fn(f64, f64) -> f64) -> Result<EdgeMin, String> {
    let a = i.bore_radius;
    let ok = |e: f64| -> Result<bool, String> {
        let (lig, peak) = per_lbf(i, e, spec)?;
        Ok(capacity(lig, peak) >= i.load)
    };
    // From e/D 0.75 (the pin searches' lower bound): a ligament much thinner than that is a sliver for the mesh.
    let (mut lo, mut hi) = (1.5 * a, 0.5 * i.width);
    if lo >= hi {
        return Ok(EdgeMin::NotSearched);
    }
    if ok(lo)? {
        return Ok(EdgeMin::AtMost(lo));
    }
    if !ok(hi)? {
        return Ok(EdgeMin::Exceeds(hi));
    }
    for _ in 0..14 {
        let mid = 0.5 * (lo + hi);
        if ok(mid)? {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Ok(EdgeMin::Value(hi))
}

fn unavailable_row(id: &'static str, label: &'static str, why: &str) -> MatrixRow {
    MatrixRow { id, label, role: Role::Comparison, error: None, unavailable: Some(why.to_string()), cells: [[None; 4]; 2], first_yield: None, p_fail_bearing: None }
}

/// The plate estimate: the plane-stress FE row, and the other models marked unavailable with the reason.
pub fn estimate(i: &PlateInput) -> Result<PlateEstimate, String> {
    validate(i)?;
    let d = 2.0 * i.bore_radius;
    let actual = i.edge / d;
    let (t, w) = (i.thickness, i.width);
    let (lig, peak) = per_lbf(i, i.edge, MeshSpec::default())?;
    let gross = 1.0 / (t * w);
    // Linear elastic: the stress per lbf scales with the load.
    let cap_net = if lig > 0.0 { i.material.ftu / lig } else { f64::INFINITY };
    let cap_yield = if peak > 0.0 { i.material.sy / peak } else { f64::INFINITY };
    let numbers = Some(PlateNumbers { net_section_capacity: cap_net, first_yield_capacity: cap_yield, kt_gross: peak / gross, ligament_over_gross: lig / gross });
    let search = MeshSpec { n_radial: 12, n_arc: [3, 4, 12], grade: 2.0 };
    let (ftu, sy) = (i.material.ftu, i.material.sy);
    let net = min_edge(i, search, |lig, _| if lig > 0.0 { ftu / lig } else { f64::INFINITY })?;
    let yld = min_edge(i, search, |_, peak| if peak > 0.0 { sy / peak } else { f64::INFINITY })?;
    let in_d = |m: EdgeMin| match m {
        EdgeMin::Value(v) => EdgeMin::Value(v / d),
        EdgeMin::AtMost(v) => EdgeMin::AtMost(v / d),
        EdgeMin::Exceeds(v) => EdgeMin::Exceeds(v / d),
        EdgeMin::NotSearched => EdgeMin::NotSearched,
    };
    let cell = |m: EdgeMin| (m != EdgeMin::NotSearched).then(|| MatrixCell::new(in_d(m), actual));
    let mut fe = unavailable_row("fem", "Plane-stress FE", "");
    fe.unavailable = None;
    fe.cells[0][0] = cell(net);
    fe.cells[1][0] = cell(yld);
    let rows = vec![
        fe,
        unavailable_row("analytic", "Stress superposition", WHY_SUPERPOSITION),
        unavailable_row("allowable", "Tabulated allowables", WHY_ALLOWABLES),
        unavailable_row("contact", "Contact FE", WHY_CONTACT),
    ];
    Ok(PlateEstimate { matrix: EdgeMatrix { target_labels: CRITERIA, actual, rows }, numbers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mechanics_core::materials::get_material;

    fn input(e_over_a: f64, w_over_a: f64, load: f64) -> PlateInput {
        let a = 0.25;
        PlateInput { bore_radius: a, edge: e_over_a * a, width: w_over_a * a, thickness: 0.125, material: Strengths::from_material(get_material("al7075")), load, angle_deg: 0.0, arm: w_over_a * a }
    }

    fn half(i: &PlateInput) -> Fields {
        solve(i.bore_radius, i.edge, i.width, i.thickness, &i.material, MeshSpec::default(), false, 0.0).unwrap()
    }

    /// A centred hole: the ligament mean is the net-section stress `P / ((W - D) t)` exactly (equilibrium of the section).
    #[test]
    fn for_a_centred_hole_the_ligament_mean_is_the_net_section_stress() {
        let i = input(8.0, 16.0, 1000.0);
        let (lig, _) = measures(&half(&i), i.bore_radius, i.edge, 1.0, 0.0).unwrap();
        let net = 1.0 / ((i.width - 2.0 * i.bore_radius) * i.thickness);
        assert!((lig / net - 1.0).abs() < 0.01, "ligament mean {lig:.6} vs net section {net:.6} per lbf");
    }

    /// A hole far from every edge: the open-hole stress concentration 3.0.
    #[test]
    fn a_hole_far_from_the_edges_has_the_open_hole_stress_concentration_of_three() {
        let i = input(40.0, 80.0, 1000.0);
        let (_, peak) = measures(&half(&i), i.bore_radius, i.edge, 1.0, 0.0).unwrap();
        let kt = peak / (1.0 / (i.thickness * i.width));
        assert!((kt / 3.0 - 1.0).abs() < 0.03, "Kt {kt:.3}");
    }

    /// The finite-width centred hole: Kt on the net section against the published Heywood / Peterson curve fit.
    #[test]
    fn a_finite_width_centred_hole_follows_the_published_stress_concentration() {
        for d_over_w in [0.2, 0.4] {
            let w = 16.0 * 0.25;
            let a = 0.5 * d_over_w * w;
            let i = PlateInput { bore_radius: a, edge: w / 2.0, width: w, thickness: 0.125, material: Strengths::from_material(get_material("al7075")), load: 1000.0, angle_deg: 0.0, arm: w };
            let (_, peak) = measures(&half(&i), a, i.edge, 1.0, 0.0).unwrap();
            // Heywood / Peterson: Kt on the net section, `3.0 - 3.13 (d/W) + 3.66 (d/W)^2 - 1.53 (d/W)^3`.
            let want = 3.0 - 3.13 * d_over_w + 3.66 * d_over_w * d_over_w - 1.53 * d_over_w.powi(3);
            let got = peak / (1.0 / ((w - 2.0 * a) * i.thickness));
            assert!((got / want - 1.0).abs() < 0.03, "d/W {d_over_w}: FE Kt on the net section {got:.3} vs the published fit {want:.3}");
        }
    }

    /// The full (mirrored) model under the axial load is the half model: 0 degrees reproduces the axial result.
    #[test]
    fn the_full_model_under_the_axial_load_reproduces_the_half_model() {
        let i = input(3.0, 16.0, 1000.0);
        let (a, e) = (i.bore_radius, i.edge);
        let (hl, hp) = measures(&half(&i), a, e, 1.0, 0.0).unwrap();
        let full = solve(a, e, i.width, i.thickness, &i.material, MeshSpec::default(), true, i.width).unwrap();
        let (fl, fp) = measures(&full, a, e, 1.0, 0.0).unwrap();
        assert!((fl / hl - 1.0).abs() < 0.01 && (fp / hp - 1.0).abs() < 0.02, "ligament {hl:.5} vs {fl:.5}, peak {hp:.5} vs {fp:.5}");
    }

    /// The reactions: a tiny hole in the middle of the strip under the transverse load with its moment gives the beam
    /// stresses of the section (the moment `V * arm` and the parabolic shear at the section).
    #[test]
    fn the_shear_case_gives_the_beam_stresses_of_the_section_so_the_reactions_balance() {
        let (w, t, a) = (4.0, 0.125, 0.04);
        let i = PlateInput { bore_radius: a, edge: w / 2.0, width: w, thickness: t, material: Strengths::from_material(get_material("al7075")), load: 1.0, angle_deg: 90.0, arm: w };
        let f = solve(a, i.edge, w, t, &i.material, MeshSpec::default(), true, i.arm).unwrap();
        let at = |x: f64, y: f64| f.at(1, x, y).unwrap();
        let x = 0.25 * w;
        // Bending: sigma_yy(x) = 12 M (x - W/2) / (t W^3) with M = V * arm at the section through the hole.
        let m = 1.0 * i.arm;
        let want_yy = 12.0 * m * (x - 0.5 * w) / (t * w.powi(3));
        // The sign is a convention of the end moment (both transverse directions are evaluated, so it never matters): the
        // magnitude must match, and the sign must be the same all along the strip.
        let got_yy = at(x, 0.0).yy;
        assert!((got_yy.abs() / want_yy.abs() - 1.0).abs() < 0.05, "bending stress {got_yy:.4} vs beam {want_yy:.4}");
        // Shear: tau(x) = 6 V x (W - x) / (t W^3).
        let want_tau = 6.0 * x * (w - x) / (t * w.powi(3));
        let got_tau = at(x, 0.0).xy.abs();
        assert!((got_tau / want_tau - 1.0).abs() < 0.05, "shear stress {got_tau:.4} vs beam {want_tau:.4}");
        // The moment grows linearly along the strip: at y = arm / 2 it is V * (arm - y) measured from the loaded end.
        let want_yy_half = 12.0 * (m - 0.5 * i.arm) * (x - 0.5 * w) / (t * w.powi(3));
        let got_yy_half = at(x, 0.5 * i.arm).yy;
        assert!(got_yy_half * got_yy > 0.0, "the bending stress keeps its sign along the strip");
        assert!((got_yy_half.abs() / want_yy_half.abs() - 1.0).abs() < 0.08, "bending stress at half the arm {got_yy_half:.4} vs beam {want_yy_half:.4}");
    }

    #[test]
    fn only_the_fe_row_has_numbers_and_the_others_say_why_not() {
        let e = estimate(&input(3.0, 16.0, 500.0)).unwrap();
        let by = |id: &str| e.matrix.rows.iter().find(|r| r.id == id).unwrap();
        assert!(by("fem").unavailable.is_none() && by("fem").cells[0][0].is_some() && by("fem").cells[1][0].is_some());
        for (id, why) in [("analytic", WHY_SUPERPOSITION), ("allowable", WHY_ALLOWABLES), ("contact", WHY_CONTACT)] {
            assert_eq!(by(id).unavailable.as_deref(), Some(why));
            assert!(by(id).cells.iter().flatten().all(|c| c.is_none()), "{id}: no number is invented");
        }
        assert_eq!(e.matrix.target_labels, CRITERIA);
        assert!(e.numbers.is_some());
        assert!(e.matrix.is_advisory(), "no legacy check exists for a plate load");
    }

    /// An inclined load adds the shear and its moment: the capacity falls as the angle grows, and a vanishing angle is the
    /// axial result.
    #[test]
    fn an_inclined_load_lowers_the_capacity_and_a_vanishing_angle_is_the_axial_result() {
        let mut i = input(3.0, 16.0, 500.0);
        let axial = estimate(&i).unwrap().numbers.unwrap();
        i.angle_deg = 1e-3;
        let nearly = estimate(&i).unwrap().numbers.unwrap();
        assert!((nearly.first_yield_capacity / axial.first_yield_capacity - 1.0).abs() < 0.02, "a vanishing angle is the axial result: {nearly:?} vs {axial:?}");
        let mut prev = axial.first_yield_capacity;
        for angle in [15.0, 45.0, 90.0] {
            i.angle_deg = angle;
            let n = estimate(&i).unwrap().numbers.unwrap();
            assert!(n.first_yield_capacity < prev, "angle {angle}: capacity {:.0} should be below {prev:.0}", n.first_yield_capacity);
            prev = n.first_yield_capacity;
        }
    }

    #[test]
    fn the_minimum_edge_distance_is_where_the_capacity_equals_the_load() {
        let i = input(3.0, 16.0, 11000.0);
        let e = estimate(&i).unwrap();
        let fe = &e.matrix.rows[0];
        let EdgeMin::Value(v) = fe.cells[1][0].unwrap().min else { panic!("first yield {:?}", fe.cells[1][0]) };
        let d = 2.0 * i.bore_radius;
        let at = PlateInput { edge: v * d, ..i.clone() };
        let n = estimate(&at).unwrap().numbers.unwrap();
        assert!((n.first_yield_capacity / i.load - 1.0).abs() < 0.04, "capacity {:.1} lbf at e/D {v:.3} vs load {:.1}", n.first_yield_capacity, i.load);
    }

    #[test]
    fn bad_inputs_are_refused() {
        let mut i = input(3.0, 16.0, 500.0);
        i.edge = 0.2;
        assert!(estimate(&i).unwrap_err().contains("exceed the bore radius"));
        let i = input(9.0, 16.0, 500.0);
        assert!(estimate(&i).unwrap_err().contains("half the plate width"));
        let mut i = input(3.0, 16.0, 500.0);
        i.angle_deg = 120.0;
        assert!(estimate(&i).unwrap_err().contains("between 0"));
        i.angle_deg = 30.0;
        i.arm = 1.0;
        assert!(estimate(&i).unwrap_err().contains("at least one plate width"));
    }
}
