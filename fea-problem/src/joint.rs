//! Finite-element compliance of a clamped bolt-joint member stack.
//!
//! The stack is an axisymmetric solid of revolution: one ring per member (own thickness, modulus,
//! Poisson's ratio, hole and outer diameter), rings sharing nodes where they touch, loaded by the
//! bolt head's (and nut's) bearing annulus with equal and opposite uniform pressure. The member
//! compliance is the energy-conjugate one, `C = 2 U / F^2`, the quantity a pressure-cone model
//! (`fastened-joint-solver::compliance::member_stack_compliance`) estimates. The bolt itself is
//! not meshed: it only carries the load, as in the cone model.

use fea_core::loads::{Loads, SurfaceLoad};
use fea_core::{Elastic, ElementKind, Mesh, Model, Physics};
use std::collections::HashMap;

/// One member of the stack, head side first. Lengths and moduli in any consistent unit system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub thickness: f64,
    pub e: f64,
    pub nu: f64,
    pub hole_diameter: f64,
    /// `None`: a large plate (the model uses a finite radius, see [`MemberFe::outer_radius_used`]).
    pub outer_diameter: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberFe {
    /// Member compliance `delta / F` (length per force).
    pub compliance: f64,
    pub elements: usize,
    pub dofs: usize,
    /// Outer radius given to a layer that has none.
    pub outer_radius_used: f64,
    pub ms: f64,
}

/// Element edge length target as a fraction of the stack thickness.
const SIZE_FRACTION: f64 = 0.12;

/// FE compliance of `layers` under bearing annuli of outer diameter `head_diameter` (head side) and
/// `nut_diameter` (nut side). Each annulus runs from its member's hole to the diameter, clipped to
/// the member's outer diameter.
pub fn member_compliance(layers: &[Layer], head_diameter: f64, nut_diameter: f64) -> Result<MemberFe, String> {
    member_compliance_sized(layers, head_diameter, nut_diameter, SIZE_FRACTION)
}

pub fn member_compliance_sized(layers: &[Layer], head_diameter: f64, nut_diameter: f64, size_fraction: f64) -> Result<MemberFe, String> {
    let clock = std::time::Instant::now();
    if layers.is_empty() {
        return Err("the stack has no members".into());
    }
    for (i, l) in layers.iter().enumerate() {
        let od_ok = l.outer_diameter.is_none_or(|od| od.is_finite() && od > l.hole_diameter);
        if !(l.thickness > 0.0 && l.e > 0.0 && l.nu > -1.0 && l.nu < 0.5 && l.hole_diameter >= 0.0 && od_ok) {
            return Err(format!("member {} has an invalid thickness, modulus, Poisson's ratio or diameter", i + 1));
        }
    }
    if !(head_diameter > 0.0 && nut_diameter > 0.0) {
        return Err("the bearing diameters must be positive".into());
    }
    let total: f64 = layers.iter().map(|l| l.thickness).sum();
    let r_big = 0.5 * head_diameter.max(nut_diameter) + 2.0 * total;
    let radii: Vec<(f64, f64)> = layers.iter().map(|l| (0.5 * l.hole_diameter, l.outer_diameter.map_or(r_big.max(0.5 * l.hole_diameter + 2.0 * total), |d| 0.5 * d))).collect();
    let head_ring = (radii[0].0, (0.5 * head_diameter).min(radii[0].1));
    let last = radii.len() - 1;
    let nut_ring = (radii[last].0, (0.5 * nut_diameter).min(radii[last].1));
    if !(head_ring.1 > head_ring.0 && nut_ring.1 > nut_ring.0) {
        return Err("a bearing annulus has no width (the bearing diameter does not exceed the hole)".into());
    }

    // Radial lattice breakpoints: every hole, outer and bearing radius, refined to the target size.
    let h = (size_fraction * total).max(1e-12);
    let mut marks: Vec<f64> = radii.iter().flat_map(|&(a, b)| [a, b]).chain([head_ring.1, nut_ring.1]).collect();
    marks.sort_by(f64::total_cmp);
    marks.dedup_by(|a, b| (*a - *b).abs() < 1e-9 * r_big);
    let mut rs = vec![marks[0]];
    for w in marks.windows(2) {
        let n = (((w[1] - w[0]) / h).ceil() as usize).max(1);
        for k in 1..=n {
            rs.push(w[0] + (w[1] - w[0]) * k as f64 / n as f64);
        }
    }
    // Axial rows per layer.
    let mut ys = vec![0.0];
    let mut layer_rows = Vec::new();
    for l in layers {
        let n = ((l.thickness / h).ceil() as usize).max(2);
        let y0 = *ys.last().unwrap();
        for k in 1..=n {
            ys.push(y0 + l.thickness * k as f64 / n as f64);
        }
        layer_rows.push(n);
    }
    // Lattice: two points per element in each direction (midside nodes at midpoints).
    let lat = |v: &[f64]| -> Vec<f64> {
        let mut out = Vec::with_capacity(2 * v.len() - 1);
        for w in v.windows(2) {
            out.push(w[0]);
            out.push(0.5 * (w[0] + w[1]));
        }
        out.push(*v.last().unwrap());
        out
    };
    let (xl, yl) = (lat(&rs), lat(&ys));

    let mut mesh = Mesh::new(Physics::Axisymmetric);
    let mut ids: HashMap<(usize, usize), usize> = HashMap::new();
    let mut node = |mesh: &mut Mesh, ix: usize, iy: usize| -> usize { *ids.entry((ix, iy)).or_insert_with(|| mesh.add_node([xl[ix], yl[iy], 0.0])) };
    let mut row0 = 0usize;
    let (mut head_faces, mut nut_faces) = (Vec::new(), Vec::new());
    for (k, l) in layers.iter().enumerate() {
        let (a, b) = radii[k];
        let mut conn = Vec::new();
        for j in row0..row0 + layer_rows[k] {
            for i in 0..rs.len() - 1 {
                let rc = 0.5 * (rs[i] + rs[i + 1]);
                if rc < a || rc > b {
                    continue;
                }
                let (x, y) = (2 * i, 2 * j);
                let pts = [(x, y), (x + 2, y), (x + 2, y + 2), (x, y + 2), (x + 1, y), (x + 2, y + 1), (x + 1, y + 2), (x, y + 1), (x + 1, y + 1)];
                for (ix, iy) in pts {
                    conn.push(node(&mut mesh, ix, iy));
                }
                // Loaded faces (body on the left): along the head face walking +r, along the nut face walking -r; midside last.
                if k == 0 && j == 0 && rc >= head_ring.0 && rc <= head_ring.1 {
                    head_faces.push(vec![node(&mut mesh, x, y), node(&mut mesh, x + 2, y), node(&mut mesh, x + 1, y)]);
                }
                if k == last && j == row0 + layer_rows[k] - 1 && rc >= nut_ring.0 && rc <= nut_ring.1 {
                    nut_faces.push(vec![node(&mut mesh, x + 2, y + 2), node(&mut mesh, x, y + 2), node(&mut mesh, x + 1, y + 2)]);
                }
            }
        }
        row0 += layer_rows[k];
        if conn.is_empty() {
            return Err(format!("member {} has no radial extent", k + 1));
        }
        mesh.add_block(ElementKind::Quad9, conn, Elastic::new(l.e, l.nu), &format!("member{}", k + 1))?;
    }
    if head_faces.is_empty() || nut_faces.is_empty() {
        return Err("a bearing annulus is too narrow to carry any element face".into());
    }
    let model = Model::new(mesh)?;
    // Unit total force F = 1 on each annulus (pressure into the stack): C = 2 U / F^2 = 2 U.
    let area = |ring: (f64, f64)| std::f64::consts::PI * (ring.1 * ring.1 - ring.0 * ring.0);
    let (ph, pn) = (1.0 / area(head_ring), 1.0 / area(nut_ring));
    let loads = Loads {
        faces: head_faces.into_iter().map(|f| (f, SurfaceLoad::Pressure(ph))).chain(nut_faces.into_iter().map(|f| (f, SurfaceLoad::Pressure(pn)))).collect(),
        ..Loads::default()
    };
    let mut bc = model.dirichlet();
    bc.fix(0, 1, 0.0); // the one free rigid motion of a revolved body is the axial translation
    let sol = model.solve_static_with(&loads, &bc, fea_core::SolveMethod::Direct)?;
    // The head and nut loads balance only if the pressures integrate to equal forces: check, not assume.
    let net = (0..model.mesh.nodes.len()).map(|n| sol.reactions[n * 2 + 1]).sum::<f64>();
    if net.abs() > 1e-6 {
        return Err(format!("the head and nut loads do not balance (net {net:.2e}): the bearing faces are mismatched"));
    }
    let u = model.strain_energy(&sol.u)?;
    Ok(MemberFe { compliance: 2.0 * u, elements: model.mesh.n_elems(), dofs: model.mesh.n_dofs(), outer_radius_used: r_big, ms: clock.elapsed().as_secs_f64() * 1e3 })
}
