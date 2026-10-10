//! Flat, axis-aligned rectangular MITC4 Mindlin plates/shells (five DOFs/node).
//! Membrane is plane stress; bending/shear use independent rotations and edge-tied shear.
//! Curved/distorted shells, drilling rotations, large rotation and laminate laws are unsupported.
use crate::fields::{DofMap, Field, FieldAssembly, FieldConstraints, FieldResult};
const FIELDS: [Field; 5] = [
    Field::Translation(0),
    Field::Translation(1),
    Field::Translation(2),
    Field::Rotation(0),
    Field::Rotation(1),
];
#[derive(Debug, Clone, Copy)]
pub struct PlateMaterial {
    pub e: f64,
    pub nu: f64,
    pub thickness: f64,
    pub shear_correction: f64,
}
impl PlateMaterial {
    fn validate(self) -> Result<(), String> {
        if [self.e, self.thickness, self.shear_correction]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || !self.nu.is_finite()
            || self.nu <= -1.0
            || self.nu >= 0.5
        {
            return Err("plate: invalid isotropic material/thickness/shear correction".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct PlateModel {
    nodes: Vec<[f64; 3]>,
    quads: Vec<[usize; 4]>,
    material: PlateMaterial,
    map: DofMap,
}
#[derive(Debug, Clone)]
pub struct PlateRecovery {
    pub membrane: [f64; 3],
    pub moments: [f64; 3],
    pub shear: [f64; 2],
}
#[derive(Debug, Clone)]
pub struct PlateSolution {
    pub fields: FieldResult,
    pub centers: Vec<PlateRecovery>,
}
fn shape(x: f64, y: f64, hx: f64, hy: f64) -> ([f64; 4], [[f64; 2]; 4]) {
    let signs = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
    let mut n = [0.0; 4];
    let mut grad = [[0.0; 2]; 4];
    for i in 0..4 {
        let [a, b] = signs[i];
        n[i] = (1.0 + a * x) * (1.0 + b * y) / 4.0;
        grad[i] = [
            a * (1.0 + b * y) / (4.0 * hx),
            b * (1.0 + a * x) / (4.0 * hy),
        ];
    }
    (n, grad)
}
fn strains(x: f64, y: f64, hx: f64, hy: f64) -> ([[f64; 20]; 3], [[f64; 20]; 3], [[f64; 20]; 2]) {
    let (_, grad) = shape(x, y, hx, hy);
    let mut membrane = [[0.0; 20]; 3];
    let mut bend = [[0.0; 20]; 3];
    let mut shear = [[0.0; 20]; 2];
    for i in 0..4 {
        let [dx, dy] = grad[i];
        let o = i * 5;
        membrane[0][o] = dx;
        membrane[1][o + 1] = dy;
        membrane[2][o] = dy;
        membrane[2][o + 1] = dx;
        bend[0][o + 4] = dx;
        bend[1][o + 3] = -dy;
        bend[2][o + 4] = dy;
        bend[2][o + 3] = -dx;
    }
    // MITC4 covariant shear tying, simplified only for validated rectangular geometry.
    for edge in [-1.0, 1.0] {
        let (n, g) = shape(0.0, edge, hx, hy);
        let weight = (1.0 + edge * y) / 2.0;
        for i in 0..4 {
            shear[0][i * 5 + 2] += weight * g[i][0];
            shear[0][i * 5 + 4] += weight * n[i];
        }
        let (n, g) = shape(edge, 0.0, hx, hy);
        let weight = (1.0 + edge * x) / 2.0;
        for i in 0..4 {
            shear[1][i * 5 + 2] += weight * g[i][1];
            shear[1][i * 5 + 3] -= weight * n[i];
        }
    }
    (membrane, bend, shear)
}
fn law(nu: f64, scale: f64) -> [[f64; 3]; 3] {
    [
        [scale, scale * nu, 0.0],
        [scale * nu, scale, 0.0],
        [0.0, 0.0, scale * (1.0 - nu) / 2.0],
    ]
}
impl PlateModel {
    pub fn new(
        nodes: Vec<[f64; 3]>,
        quads: Vec<[usize; 4]>,
        material: PlateMaterial,
    ) -> Result<Self, String> {
        material.validate()?;
        if nodes.is_empty() || quads.is_empty() || nodes.iter().flatten().any(|v| !v.is_finite()) {
            return Err("plate: empty model or nonfinite coordinates".into());
        }
        let map = DofMap::uniform(nodes.len(), &FIELDS)?;
        let model = Self {
            nodes,
            quads,
            material,
            map,
        };
        for quad in &model.quads {
            model.geometry(quad)?;
        }
        Ok(model)
    }
    pub fn fields(&self) -> &DofMap {
        &self.map
    }
    pub fn nodes(&self) -> &[[f64; 3]] {
        &self.nodes
    }
    fn geometry(&self, quad: &[usize; 4]) -> Result<(f64, f64), String> {
        if quad.iter().any(|&n| n >= self.nodes.len()) {
            return Err("plate: invalid connectivity".into());
        }
        let [a, b, c, d] = quad.map(|n| self.nodes[n]);
        let hx = (b[0] - a[0]) / 2.0;
        let hy = (d[1] - a[1]) / 2.0;
        if hx <= 0.0 || hy <= 0.0 || !hx.is_finite() || !hy.is_finite() {
            return Err("plate: nonpositive rectangular dimensions".into());
        }
        let expected = [
            [a[0], a[1], a[2]],
            [b[0], a[1], a[2]],
            [b[0], d[1], a[2]],
            [a[0], d[1], a[2]],
        ];
        let tol = 1e-12 * (2.0 * hx).max(2.0 * hy);
        for (actual, want) in [a, b, c, d].iter().zip(expected) {
            for axis in 0..3 {
                if (actual[axis] - want[axis]).abs() > tol {
                    return Err("plate: only flat axis-aligned rectangles in counterclockwise order are supported".into());
                }
            }
        }
        Ok((hx, hy))
    }
    pub fn solve(
        &self,
        bc: &FieldConstraints,
        loads: &[(usize, Field, f64)],
        pressure: &dyn Fn([f64; 3]) -> f64,
        tol: f64,
    ) -> Result<PlateSolution, String> {
        crate::validate::check_field_supports(
            &self.nodes,
            &self.map,
            &self.quads.iter().map(|q| q.to_vec()).collect::<Vec<_>>(),
            bc,
        )?;
        let mat = self.material;
        let dm = law(mat.nu, mat.e * mat.thickness / (1.0 - mat.nu * mat.nu));
        let db = law(
            mat.nu,
            mat.e * mat.thickness.powi(3) / (12.0 * (1.0 - mat.nu * mat.nu)),
        );
        let ds = mat.shear_correction * mat.e * mat.thickness / (2.0 * (1.0 + mat.nu));
        let mut assembly = FieldAssembly::new(self.map.clone());
        let mut element_dofs = Vec::new();
        let point = 1.0_f64 / 3.0_f64.sqrt();
        for quad in &self.quads {
            let (hx, hy) = self.geometry(quad)?;
            let dofs = quad
                .iter()
                .flat_map(|&n| FIELDS.into_iter().map(move |f| self.map.index(n, f)))
                .collect::<Result<Vec<_>, _>>()?;
            let mut k = vec![0.0; 400];
            let mut force = [0.0; 20];
            for x in [-point, point] {
                for y in [-point, point] {
                    let (bm, bb, bs) = strains(x, y, hx, hy);
                    let (n, _) = shape(x, y, hx, hy);
                    let mut location = [0.0; 3];
                    for a in 0..4 {
                        for axis in 0..3 {
                            location[axis] += n[a] * self.nodes[quad[a]][axis];
                        }
                    }
                    let q = pressure(location);
                    if !q.is_finite() {
                        return Err("plate: nonfinite pressure".into());
                    }
                    for a in 0..4 {
                        force[a * 5 + 2] += n[a] * q * hx * hy;
                    }
                    for i in 0..20 {
                        for j in 0..20 {
                            let mut value = 0.0;
                            for a in 0..3 {
                                for b in 0..3 {
                                    value += bm[a][i] * dm[a][b] * bm[b][j]
                                        + bb[a][i] * db[a][b] * bb[b][j];
                                }
                            }
                            for a in 0..2 {
                                value += ds * bs[a][i] * bs[a][j];
                            }
                            k[i * 20 + j] += value * hx * hy;
                        }
                    }
                }
            }
            assembly.add_element(&dofs, &k)?;
            for (&d, &f) in dofs.iter().zip(&force) {
                assembly.add_load(d, f)?;
            }
            element_dofs.push(dofs);
        }
        for &(node, field, value) in loads {
            assembly.add_load(self.map.index(node, field)?, value)?;
        }
        let fields = assembly.finish().solve(bc, tol)?;
        let mut centers = Vec::new();
        for (quad, dofs) in self.quads.iter().zip(element_dofs) {
            let (hx, hy) = self.geometry(quad)?;
            let (bm, bb, bs) = strains(0.0, 0.0, hx, hy);
            let u: Vec<f64> = dofs.iter().map(|&d| fields.values[d]).collect();
            let eps: [f64; 3] = std::array::from_fn(|a| (0..20).map(|i| bm[a][i] * u[i]).sum());
            let curv: [f64; 3] = std::array::from_fn(|a| (0..20).map(|i| bb[a][i] * u[i]).sum());
            let membrane = std::array::from_fn(|a| (0..3).map(|b| dm[a][b] * eps[b]).sum());
            let moments = std::array::from_fn(|a| (0..3).map(|b| db[a][b] * curv[b]).sum());
            let shear = std::array::from_fn(|a| ds * (0..20).map(|i| bs[a][i] * u[i]).sum::<f64>());
            centers.push(PlateRecovery {
                membrane,
                moments,
                shear,
            });
        }
        Ok(PlateSolution { fields, centers })
    }
}
