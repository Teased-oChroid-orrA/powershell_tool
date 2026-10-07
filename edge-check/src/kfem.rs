//! The plane-stress / plane-strain plate with a bore near a free edge on the general FE kernel
//! (`fea-core`): the same O-grid of Q9 elements as [`crate::fem::FemSolution`] (shared through
//! `fem::plate_mesh`), the same three unit load cases (uniform fit pressure, cosine pin load on the
//! loaded half, cosine pin load all round) solved against one factorisation, and stress at any
//! point by the kernel's point locator. `FemSolution` stays for the contact and plastic collapse
//! solvers, which reuse its banded factorisation as an iteration matrix.

use crate::fem::{plate_mesh, MeshSpec, PlateMesh, UnitCase};
use crate::types::{Geometry, Stress};
use fea_core::loads::{FaceField, Loads, SurfaceLoad};
use fea_core::locate::Locator;
use fea_core::{Dirichlet, Elastic, ElementKind, Mesh, Model, Physics};
use std::f64::consts::PI;

/// `vtk[k] = ours[PERM[k]]`: this crate's Q9 local order is `3 * b + a` (row `b`, column `a`), VTK's is
/// corners, edge midsides, centre.
const PERM: [usize; 9] = [0, 2, 8, 6, 1, 5, 7, 3, 4];

pub struct KernelFem {
    model: Model,
    u: [Vec<f64>; 3],
    locator: Locator,
    centre: [f64; 2],
    bore_radius: f64,
}

impl KernelFem {
    pub fn solve(geom: &Geometry, e: f64, nu: f64, spec: MeshSpec) -> Result<Self, String> {
        Self::solve_with(geom, e, nu, spec, false)
    }

    /// `plane_strain = true` models a thick plate (`eps_zz = 0`); `e` / `nu` are the true material constants.
    pub fn solve_with(geom: &Geometry, e: f64, nu: f64, spec: MeshSpec, plane_strain: bool) -> Result<Self, String> {
        let (a, edge) = (geom.bore_radius, geom.edge);
        if !(a > 0.0 && edge > a && geom.thickness > 0.0 && geom.plate_far > a && geom.plate_half_height > a) {
            return Err("degenerate plate geometry".to_string());
        }
        let (far, h, t) = (geom.plate_far, geom.plate_half_height, geom.thickness);
        let PlateMesh { nodes, elems, na, n_theta_el } = plate_mesh(geom, spec);
        let physics = if plane_strain { Physics::PlaneStrain { thickness: t } } else { Physics::PlaneStress { thickness: t } };
        let mut mesh = Mesh::new(physics);
        for p in &nodes {
            mesh.add_node([p[0], p[1], 0.0]);
        }
        let conn: Vec<usize> = elems.iter().flat_map(|c| PERM.iter().map(move |&k| c[k])).collect();
        mesh.add_block(ElementKind::Quad9, conn, Elastic::new(e, nu), "plate")?;

        // Bore edges (lattice column 0): the body is outside the circle, so walk with decreasing angle to keep it on the left.
        let bore: Vec<Vec<usize>> = (0..n_theta_el).map(|j| vec![(2 * j + 2) * na, (2 * j) * na, (2 * j + 1) * na]).collect();
        // Far-face edges: body on the left walking up (increasing angle); only rows that lie on the face.
        let far_x = edge + far;
        let far_faces: Vec<Vec<usize>> = (0..n_theta_el)
            .map(|j| vec![(2 * j) * na + na - 1, (2 * j + 2) * na + na - 1, (2 * j + 1) * na + na - 1])
            .filter(|f| f.iter().all(|&n| (nodes[n][0] - far_x).abs() < 1e-9 * far_x.abs().max(1.0)))
            .collect();

        // Unit cases per unit pin resultant: tractions are force per area, the kernel multiplies by the thickness.
        let cx = edge;
        let cosine = move |half: bool, scale: f64| {
            FaceField::new(move |x: &[f64; 3], _n: &[f64; 3]| {
                let phi = x[1].atan2(x[0] - cx);
                let (c, s) = (phi.cos(), phi.sin());
                // Pin load cosine about the loaded point (phi = pi): p = p0 cos(phi - pi) = -p0 cos(phi).
                let cos_load = -c;
                let p = if half { if cos_load > 0.0 { 2.0 / (PI * a) * cos_load } else { 0.0 } } else { 1.0 / (PI * a) * cos_load };
                let p = p * scale;
                [p * c, p * s, 0.0]
            })
        };
        let sigma_far = 1.0 / (t * 2.0 * h);
        let pin_case = |half: bool| {
            let field = cosine(half, 1.0 / t);
            Loads {
                field_faces: bore.iter().map(|f| (f.clone(), field.clone())).collect(),
                faces: far_faces.iter().map(|f| (f.clone(), SurfaceLoad::Traction([sigma_far, 0.0, 0.0]))).collect(),
                ..Loads::default()
            }
        };
        // The fit pressure is radial about the bore centre (the true circle), not along the element edge's own
        // normal, so the load equals the one the analytic model and the banded solver apply.
        let radial = FaceField::new(move |x: &[f64; 3], _n: &[f64; 3]| {
            let phi = x[1].atan2(x[0] - cx);
            [phi.cos(), phi.sin(), 0.0]
        });
        let fit = Loads { field_faces: bore.iter().map(|f| (f.clone(), radial.clone())).collect(), ..Loads::default() };
        let cases = [fit, pin_case(true), pin_case(false)];

        let model = Model::new(mesh)?;
        // Symmetry (u_y = 0 on y = 0) and one u_x anchor on the symmetry axis at the far face.
        let mut bc: Dirichlet = model.dirichlet();
        for (n, p) in nodes.iter().enumerate() {
            if p[1] == 0.0 {
                bc.fix(n, 1, 0.0);
            }
        }
        bc.fix(na - 1, 0, 0.0);
        let sols = model.solve_static_many(&cases, &bc)?;
        let mut it = sols.into_iter().map(|s| s.u);
        let u = [it.next().unwrap(), it.next().unwrap(), it.next().unwrap()];
        let locator = model.locator();
        Ok(Self { model, u, locator, centre: [cx, 0.0], bore_radius: a })
    }

    /// Stress at `(x, y)` for unit case `case`, averaged over every element containing the point. `None`
    /// outside the mesh. The solution is symmetric about `y = 0`; negative `y` is mirrored (`xy` flips).
    pub fn stress_at(&self, case: UnitCase, x: f64, y: f64) -> Option<Stress> {
        let (y, flip) = if y < 0.0 { (-y, -1.0) } else { (y, 1.0) };
        let s = self.locator.stress_at(&self.model, &self.u[case as usize], [x, y, 0.0], 0.0)?;
        Some(Stress { xx: s[0], yy: s[1], xy: s[3] * flip })
    }

    pub fn bore_radius(&self) -> f64 {
        self.bore_radius
    }

    pub fn centre(&self) -> [f64; 2] {
        self.centre
    }

    pub fn element_count(&self) -> usize {
        self.model.mesh.n_elems()
    }

    pub fn dof_count(&self) -> usize {
        self.model.mesh.n_dofs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fem::FemSolution;

    fn geom(edge_over_d: f64) -> Geometry {
        let a = 0.25;
        Geometry { bore_radius: a, edge: edge_over_d * 2.0 * a, thickness: 0.5, plate_far: 6.0 * a * 2.0, plate_half_height: 6.0 * a * 2.0, plane_angle_deg: 40.0 }
    }

    /// The kernel solve of the same mesh reproduces the original banded solver at probe points all
    /// around the bore, across the ligament and along the shear-out plane, in every unit case.
    #[test]
    fn the_kernel_fe_matches_the_banded_fe_on_the_same_mesh() {
        for (plane_strain, spec) in [(false, MeshSpec::default()), (true, MeshSpec { n_radial: 12, n_arc: [3, 4, 12], grade: 2.0 })] {
            for ed in [1.0, 1.5, 3.0] {
                let g = geom(ed);
                let old = FemSolution::solve_with(&g, 10.3e6, 0.33, spec, plane_strain).unwrap();
                let new = KernelFem::solve_with(&g, 10.3e6, 0.33, spec, plane_strain).unwrap();
                assert_eq!((old.element_count(), old.dof_count()), (new.element_count(), new.dof_count()));
                let mut probes: Vec<(f64, f64)> = (0..=24).map(|k| {
                    let phi = PI * k as f64 / 24.0;
                    (g.edge + g.bore_radius * 1.0001 * phi.cos(), g.bore_radius * 1.0001 * phi.sin())
                }).collect();
                probes.extend((1..10).map(|k| (g.edge * k as f64 / 10.0 * 0.9, 0.0)));
                probes.extend((1..10).map(|k| (g.edge * k as f64 / 9.0, g.bore_radius)));
                probes.extend([(0.0, 0.0), (g.edge + 3.0 * g.bore_radius, -0.7 * g.bore_radius)]);
                for case in [UnitCase::Fit, UnitCase::PinHalf, UnitCase::PinFull] {
                    let scale = probes.iter().filter_map(|&(x, y)| old.stress_at(case, x, y)).map(|s| s.xx.abs().max(s.yy.abs()).max(s.xy.abs())).fold(0.0f64, f64::max);
                    for &(x, y) in &probes {
                        let (o, n) = (old.stress_at(case, x, y), new.stress_at(case, x, y));
                        let (Some(o), Some(n)) = (o, n) else { assert_eq!(o.is_some(), n.is_some(), "{case:?} ({x}, {y}) located differently"); continue };
                        for (a, b, name) in [(o.xx, n.xx, "xx"), (o.yy, n.yy, "yy"), (o.xy, n.xy, "xy")] {
                            assert!((a - b).abs() < 1e-6 * scale, "ps={plane_strain} e/D={ed} {case:?} ({x:.4}, {y:.4}) {name}: banded {a} vs kernel {b}");
                        }
                    }
                }
            }
        }
    }
}

