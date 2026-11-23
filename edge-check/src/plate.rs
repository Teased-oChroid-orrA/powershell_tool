//! A plate (a finite strip) with a hole near a free edge under an axial plate load: the plane-stress FE on `fea-core`,
//! two criteria, each its own column of the matrix:
//!
//! * **Net section** - the mean stress along the load across the ligament between the hole and the near free edge
//!   (`y = 0`, `x` from the edge to the bore) against `Ftu`. For a centred hole this equals `P / ((W - D) t)` exactly
//!   (equilibrium; tested). For a hole nearer one edge the ligament carries more than the section mean, and that is the
//!   number this criterion reports.
//! * **First yield** - the peak elastic von Mises stress on the bore and the near free edge against `Sy`
//!   (the hole's stress concentration, about 3 for a hole far from every edge; tested).
//!
//! Load: a uniform tension along the strip, parallel to the free edges, applied on the strip's ends (a uniform end traction is the exact uniform stress state of a strip with free edges, so no length is needed to lose end effects). Only that case is defined: a load at an angle to a strip with free edges is not
//! determined by a width and an angle alone (the free edges cannot carry a load perpendicular to them, and an inclined end
//! load needs a statement of how the shear and the moment are reacted), so any other angle is reported as unavailable, not
//! computed. The other models cannot represent a finite strip at all and are shown as unavailable with the reason.
//!
//! Not validated against test data: a limiting check only (the open-hole stress concentration and the equilibrium of the
//! net section). Elastic, small strain, plane stress; no hardening, no fracture, no bearing.

use crate::fem::{plate_mesh, MeshSpec, PlateMesh};
use crate::field::gauss_legendre_unit;
use crate::kfem::PERM;
use crate::matrix::{EdgeMatrix, MatrixCell, MatrixRow, Role};
use crate::runner::EdgeMin;
use crate::types::{Geometry, Strengths, Stress};
use fea_core::loads::{Loads, SurfaceLoad};
use fea_core::locate::Locator;
use fea_core::{Dirichlet, Elastic, ElementKind, Mesh, Model, Physics};
use std::f64::consts::PI;

/// Column labels of the plate matrix (instead of the pin case's applied load / bearing limit).
pub const CRITERIA: [&str; 2] = ["Net section vs Ftu", "First yield at the hole"];

pub const WHY_SUPERPOSITION: &str = "the half-plane stress superposition has one free edge and no far field: it cannot represent a finite strip";
pub const WHY_ALLOWABLES: &str = "the tabulated allowables are pin-bearing and shear-out rules: they say nothing about a plate load";
pub const WHY_CONTACT: &str = "the contact FE is built around a pin loading the bore: with no pin it is a different, unvalidated use";
pub const WHY_ANGLE: &str = "a load at an angle to a strip with free edges needs its boundary conditions stated (how the shear and the moment are reacted): only the load along the strip is defined";

#[derive(Debug, Clone, PartialEq)]
pub struct PlateInput {
    pub bore_radius: f64,
    /// Bore centre to the nearer free edge; at most half the width.
    pub edge: f64,
    /// Strip width, edge to edge.
    pub width: f64,
    pub thickness: f64,
    pub material: Strengths,
    /// Axial plate load, lbf.
    pub load: f64,
    /// Load direction against the strip axis, degrees: only `0` is defined.
    pub angle_deg: f64,
}

/// The numbers behind the FE cells.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateNumbers {
    /// Load the plate carries before the criterion fails at the actual geometry (lbf).
    pub net_section_capacity: f64,
    pub first_yield_capacity: f64,
    /// Peak von Mises on the bore and the free edge per unit gross stress `P / (W t)`: the stress concentration read
    /// against the gross section.
    pub kt_gross: f64,
    /// Mean ligament stress per unit gross stress.
    pub ligament_over_gross: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlateEstimate {
    /// The matrix with the plane-stress FE as the only available row.
    pub matrix: EdgeMatrix,
    /// `None` when the load angle is not defined (every row unavailable).
    pub numbers: Option<PlateNumbers>,
}

struct Fields {
    model: Model,
    u: Vec<f64>,
    locator: Locator,
}

impl Fields {
    /// Stress per lbf of axial load at `(x, y)` (`y >= 0`; the solution is symmetric about `y = 0`).
    fn at(&self, x: f64, y: f64) -> Option<Stress> {
        let s = self.locator.stress_at(&self.model, &self.u, [x, y, 0.0], 0.0)?;
        Some(Stress { xx: s[0], yy: s[1], xy: s[3] })
    }
}

fn solve(a: f64, e: f64, w: f64, t: f64, mat: &Strengths, spec: MeshSpec) -> Result<Fields, String> {
    // A uniform end traction on a strip with free edges is the exact uniform stress state, so the strip needs no
    // length to lose its end effects; its half length only has to leave the mesh well shaped.
    let h = (w - e).max(8.0 * a);
    let geom = Geometry { bore_radius: a, edge: e, thickness: t, plate_far: w - e, plate_half_height: h, plane_angle_deg: 40.0 };
    let PlateMesh { nodes, elems, na, n_theta_el } = plate_mesh(&geom, spec);
    let mut mesh = Mesh::new(Physics::PlaneStress { thickness: t });
    for p in &nodes {
        mesh.add_node([p[0], p[1], 0.0]);
    }
    let conn: Vec<usize> = elems.iter().flat_map(|c| PERM.iter().map(move |&k| c[k])).collect();
    mesh.add_block(ElementKind::Quad9, conn, Elastic::new(mat.e, mat.nu), "plate")?;
    // The strip's end (y = h): the outer ring edges lying on it, body on the left walking up the ring.
    let top: Vec<Vec<usize>> = (0..n_theta_el)
        .map(|j| vec![(2 * j) * na + na - 1, (2 * j + 2) * na + na - 1, (2 * j + 1) * na + na - 1])
        .filter(|f| f.iter().all(|&n| (nodes[n][1] - h).abs() < 1e-9 * h))
        .collect();
    if top.is_empty() {
        return Err("the strip's end face was not found in the mesh".into());
    }
    // 1 lbf over the end: force per area times the thickness.
    let sigma = 1.0 / (t * w);
    let loads = Loads { faces: top.iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, sigma, 0.0]))).collect(), ..Loads::default() };
    let model = Model::new(mesh)?;
    // Symmetry about the load line through the bore (u_y = 0 on y = 0) and one u_x anchor on it at the far face.
    let mut bc: Dirichlet = model.dirichlet();
    for (n, p) in nodes.iter().enumerate() {
        if p[1] == 0.0 {
            bc.fix(n, 1, 0.0);
        }
    }
    bc.fix(na - 1, 0, 0.0);
    let u = model.solve_static_many(&[loads], &bc)?.into_iter().next().ok_or("no solution")?.u;
    let locator = model.locator();
    Ok(Fields { model, u, locator })
}

/// `(mean ligament stress, peak von Mises)` per lbf.
fn measures(f: &Fields, a: f64, e: f64) -> Result<(f64, f64), String> {
    let miss = |what: &str, x: f64, y: f64| format!("stress field has no value at the {what} probe ({x:.4}, {y:.4})");
    let mut ligament = 0.0;
    for (u, w) in gauss_legendre_unit(24) {
        let x = (e - a) * u;
        ligament += w * f.at(x, 0.0).ok_or_else(|| miss("ligament", x, 0.0))?.yy;
    }
    let mut peak: f64 = 0.0;
    const NB: usize = 73;
    for k in 0..NB {
        let phi = PI * k as f64 / (NB - 1) as f64;
        let (x, y) = (e + a * phi.cos(), a * phi.sin());
        peak = peak.max(f.at(x, y).ok_or_else(|| miss("bore", x, y))?.von_mises());
    }
    const NE: usize = 25;
    for k in 0..NE {
        let y = 4.0 * a * ((k as f64) / (NE - 1) as f64).powi(2);
        peak = peak.max(f.at(0.0, y).ok_or_else(|| miss("free-edge", 0.0, y))?.von_mises());
    }
    Ok((ligament, peak))
}

/// Why the input cannot be estimated, if it cannot.
pub fn validate(i: &PlateInput) -> Result<(), String> {
    let (a, e, w, t) = (i.bore_radius, i.edge, i.width, i.thickness);
    if !(a > 0.0 && a.is_finite() && t > 0.0 && t.is_finite() && w.is_finite() && e.is_finite()) {
        return Err("bore diameter, plate width and thickness must be finite and positive".into());
    }
    if !(i.load.is_finite() && i.load >= 0.0 && i.angle_deg.is_finite()) {
        return Err("the load must be finite and not negative".into());
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
    Ok(())
}

/// The smallest edge distance (in) at which the criterion is met at `load`, between e/D 0.75 and half the width. The margin rises with the edge distance on that range.
fn min_edge(i: &PlateInput, spec: MeshSpec, capacity: impl Fn(f64, f64) -> f64) -> Result<EdgeMin, String> {
    let (a, w, t, mat) = (i.bore_radius, i.width, i.thickness, i.material);
    let ok = |e: f64| -> Result<bool, String> {
        let f = solve(a, e, w, t, &mat, spec)?;
        let (lig, peak) = measures(&f, a, e)?;
        Ok(capacity(lig, peak) >= i.load)
    };
    // From e/D 0.75 (the pin searches' lower bound): a ligament much thinner than that is a sliver for the mesh.
    let (mut lo, mut hi) = (1.5 * a, 0.5 * w);
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
    let mut fe = unavailable_row("fem", "Plane-stress FE", WHY_ANGLE);
    let mut numbers = None;
    if i.angle_deg.abs() < 1e-9 {
        let (a, e, w, t) = (i.bore_radius, i.edge, i.width, i.thickness);
        let f = solve(a, e, w, t, &i.material, MeshSpec::default())?;
        let (lig, peak) = measures(&f, a, e)?;
        let gross = 1.0 / (t * w);
        // Linear elastic: the stress per lbf scales with the load.
        let cap_net = if lig > 0.0 { i.material.ftu / lig } else { f64::INFINITY };
        let cap_yield = if peak > 0.0 { i.material.sy / peak } else { f64::INFINITY };
        numbers = Some(PlateNumbers { net_section_capacity: cap_net, first_yield_capacity: cap_yield, kt_gross: peak / gross, ligament_over_gross: lig / gross });
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
        fe.unavailable = None;
        fe.cells[0][0] = cell(net);
        fe.cells[1][0] = cell(yld);
    }
    let rows = vec![
        fe,
        unavailable_row("analytic", "Stress superposition", if i.angle_deg.abs() < 1e-9 { WHY_SUPERPOSITION } else { WHY_ANGLE }),
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
        PlateInput { bore_radius: a, edge: e_over_a * a, width: w_over_a * a, thickness: 0.125, material: Strengths::from_material(get_material("al7075")), load, angle_deg: 0.0 }
    }

    /// A centred hole: the ligament mean is the net-section stress `P / ((W - D) t)` exactly (equilibrium of the section).
    #[test]
    fn for_a_centred_hole_the_ligament_mean_is_the_net_section_stress() {
        let i = input(8.0, 16.0, 1000.0);
        let (a, e, w, t) = (i.bore_radius, i.edge, i.width, i.thickness);
        let f = solve(a, e, w, t, &i.material, MeshSpec::default()).unwrap();
        let (lig, _) = measures(&f, a, e).unwrap();
        let net = 1.0 / ((w - 2.0 * a) * t);
        assert!((lig / net - 1.0).abs() < 0.01, "ligament mean {lig:.6} vs net section {net:.6} per lbf");
    }

    /// A hole far from every edge: the stress concentration is the open-hole 3.0 (peak von Mises at the bore, uniaxial
    /// state, equals the peak axial stress).
    #[test]
    fn a_hole_far_from_the_edges_has_the_open_hole_stress_concentration_of_three() {
        let i = input(40.0, 80.0, 1000.0);
        let (a, e, w, t) = (i.bore_radius, i.edge, i.width, i.thickness);
        let f = solve(a, e, w, t, &i.material, MeshSpec::default()).unwrap();
        let (_, peak) = measures(&f, a, e).unwrap();
        let kt = peak / (1.0 / (t * w));
        assert!((kt / 3.0 - 1.0).abs() < 0.03, "Kt {kt:.3}");
    }

    /// The finite-width centred hole: Kt on the net section against the published Heywood / Peterson curve fit.
    #[test]
    fn a_finite_width_centred_hole_follows_the_published_stress_concentration() {
        for d_over_w in [0.2, 0.4] {
            let w = 16.0 * 0.25;
            let a = 0.5 * d_over_w * w;
            let i = PlateInput { bore_radius: a, edge: w / 2.0, width: w, thickness: 0.125, material: Strengths::from_material(get_material("al7075")), load: 1000.0, angle_deg: 0.0 };
            let f = solve(a, i.edge, w, i.thickness, &i.material, MeshSpec::default()).unwrap();
            let (_, peak) = measures(&f, a, i.edge).unwrap();
            // Heywood / Peterson: Kt on the net section, `3.0 - 3.13 (d/W) + 3.66 (d/W)^2 - 1.53 (d/W)^3`.
            let want = 3.0 - 3.13 * d_over_w + 3.66 * d_over_w * d_over_w - 1.53 * d_over_w.powi(3);
            let got = peak / (1.0 / ((w - 2.0 * a) * i.thickness));
            assert!((got / want - 1.0).abs() < 0.03, "d/W {d_over_w}: FE Kt on the net section {got:.3} vs the published fit {want:.3}");
        }
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

    #[test]
    fn a_load_at_an_angle_is_unavailable_everywhere_not_computed() {
        let mut i = input(3.0, 16.0, 500.0);
        i.angle_deg = 30.0;
        let e = estimate(&i).unwrap();
        assert!(e.numbers.is_none());
        assert!(e.matrix.rows.iter().all(|r| r.unavailable.is_some() && r.cells.iter().flatten().all(|c| c.is_none())));
        assert_eq!(e.matrix.rows[0].unavailable.as_deref(), Some(WHY_ANGLE));
    }

    #[test]
    fn the_minimum_edge_distance_is_where_the_capacity_equals_the_load() {
        let i = input(3.0, 16.0, 11000.0);
        let e = estimate(&i).unwrap();
        let fe = &e.matrix.rows[0];
        let EdgeMin::Value(v) = fe.cells[1][0].unwrap().min else { panic!("first yield {:?}", fe.cells[1][0]) };
        let d = 2.0 * i.bore_radius;
        // At that edge distance the first-yield capacity is the load (fine mesh within the search mesh's error).
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
    }
}


