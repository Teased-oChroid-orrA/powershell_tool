//! Mesh, material and analysis-type definitions.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::element::ElementKind;
use crate::material::J2;
use std::collections::BTreeMap;

/// What the 2D elements represent. Coordinates are `(x, y)` (or `(r, z)` axisymmetric); the
/// `z` component of a node is ignored in 2D.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Physics {
    /// `sigma_zz = 0`; the model has the given thickness (force = traction x thickness).
    PlaneStress { thickness: f64 },
    /// `eps_zz = 0`; per the given thickness (1 = per unit thickness).
    PlaneStrain { thickness: f64 },
    /// Solid of revolution about the `z` axis, `x` = radius; loads are for the full 360 degrees.
    Axisymmetric,
    /// Three-dimensional solid.
    Solid,
}

impl Physics {
    /// Spatial dimension of the displacement field (degrees of freedom per node).
    pub fn dim(self) -> usize {
        if matches!(self, Physics::Solid) {
            3
        } else {
            2
        }
    }
}

/// Fully anisotropic linear elasticity: stiffness `d` and its inverse (compliance) `c`, `6 x 6` in
/// the library stress order `(xx, yy, zz, xy, yz, zx)` with engineering shear strains. In plane
/// problems the material must be symmetric about `z` (no coupling of `yz`/`zx` with the in-plane
/// components); axisymmetric problems read the axes as `(r, z, theta)` = `(x, y, z)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aniso {
    pub d: [[f64; 6]; 6],
    pub c: [[f64; 6]; 6],
    /// Thermal expansion tensor of the anisotropic material, per degree, as the strain vector `(a_xx, a_yy, a_zz, 2 a_xy,
    /// 2 a_yz, 2 a_zx)` (engineering shear). `None`: the isotropic `Elastic::alpha` on the three normal components.
    pub alpha: Option<[f64; 6]>,
}

/// Linear elastic material (psi, inch units by convention; the kernel is unit-free). Isotropic
/// unless `aniso` is set, in which case `e` / `nu` hold only a Voigt-average equivalent for display
/// and the constitutive law is `aniso`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elastic {
    pub e: f64,
    pub nu: f64,
    /// Coefficient of thermal expansion (per degree), used only with a temperature change. The
    /// thermal strain is isotropic, `alpha dT` on the three normal components.
    pub alpha: f64,
    pub aniso: Option<Aniso>,
}

/// Invert a symmetric positive definite `6 x 6` matrix by Cholesky; `None` if it is not SPD.
fn spd_inverse(a: &[[f64; 6]; 6]) -> Option<[[f64; 6]; 6]> {
    let mut l = [[0.0f64; 6]; 6];
    for i in 0..6 {
        for j in 0..=i {
            let mut s = a[i][j];
            for k in 0..j {
                s -= l[i][k] * l[j][k];
            }
            if i == j {
                if s.is_nan() || s <= 1e-14 * a[i][i].abs().max(1e-300) {
                    return None;
                }
                l[i][i] = s.sqrt();
            } else {
                l[i][j] = s / l[j][j];
            }
        }
    }
    let mut inv = [[0.0f64; 6]; 6];
    for col in 0..6 {
        let mut y = [0.0f64; 6];
        for i in 0..6 {
            let mut s = if i == col { 1.0 } else { 0.0 };
            for k in 0..i {
                s -= l[i][k] * y[k];
            }
            y[i] = s / l[i][i];
        }
        for i in (0..6).rev() {
            let mut s = y[i];
            for k in i + 1..6 {
                s -= l[k][i] * inv[k][col];
            }
            inv[i][col] = s / l[i][i];
        }
    }
    Some(inv)
}

impl Elastic {
    pub fn new(e: f64, nu: f64) -> Self {
        Self { e, nu, alpha: 0.0, aniso: None }
    }

    /// General anisotropic material from its `6 x 6` stiffness (symmetric positive definite).
    pub fn anisotropic(d: [[f64; 6]; 6]) -> Result<Self, String> {
        for i in 0..6 {
            for j in 0..i {
                if (d[i][j] - d[j][i]).abs() > 1e-9 * (d[i][i].abs() + d[j][j].abs()) {
                    return Err("the stiffness matrix must be symmetric".into());
                }
            }
        }
        let c = spd_inverse(&d).ok_or("the stiffness matrix is not positive definite")?;
        // Voigt-average equivalents (display and any isotropic-only consumer).
        let k = ((d[0][0] + d[1][1] + d[2][2]) + 2.0 * (d[0][1] + d[1][2] + d[2][0])) / 9.0;
        let g = ((d[0][0] + d[1][1] + d[2][2]) - (d[0][1] + d[1][2] + d[2][0]) + 3.0 * (d[3][3] + d[4][4] + d[5][5])) / 15.0;
        let (e, nu) = (9.0 * k * g / (3.0 * k + g), (3.0 * k - 2.0 * g) / (2.0 * (3.0 * k + g)));
        Ok(Self { e, nu, alpha: 0.0, aniso: Some(Aniso { d, c, alpha: None }) })
    }

    /// Orthotropic material with principal axes along `x, y, z` from engineering constants
    /// (`nu_ij` = strain in `j` per strain in `i`, `g12 = G_xy`, `g23 = G_yz`, `g13 = G_zx`).
    #[allow(clippy::too_many_arguments)] // the nine independent engineering constants
    pub fn orthotropic(e1: f64, e2: f64, e3: f64, nu12: f64, nu13: f64, nu23: f64, g12: f64, g23: f64, g13: f64) -> Result<Self, String> {
        let mut s = [[0.0f64; 6]; 6];
        s[0][0] = 1.0 / e1;
        s[1][1] = 1.0 / e2;
        s[2][2] = 1.0 / e3;
        s[0][1] = -nu12 / e1;
        s[1][0] = s[0][1];
        s[0][2] = -nu13 / e1;
        s[2][0] = s[0][2];
        s[1][2] = -nu23 / e2;
        s[2][1] = s[1][2];
        s[3][3] = 1.0 / g12;
        s[4][4] = 1.0 / g23;
        s[5][5] = 1.0 / g13;
        let d = spd_inverse(&s).ok_or("the engineering constants do not give a positive definite material")?;
        Self::anisotropic(d)
    }

    /// The same material with its principal axes rotated by `phi` (radians) about `z`.
    pub fn rotated_z(&self, phi: f64) -> Result<Self, String> {
        let an = self.aniso.ok_or("only an anisotropic material can be rotated")?;
        let (c, s) = (phi.cos(), phi.sin());
        let r = [
            [c * c, s * s, 0.0, -2.0 * c * s, 0.0, 0.0],
            [s * s, c * c, 0.0, 2.0 * c * s, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            [c * s, -c * s, 0.0, c * c - s * s, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0, c, s],
            [0.0, 0.0, 0.0, 0.0, -s, c],
        ];
        let mut d = [[0.0f64; 6]; 6];
        for i in 0..6 {
            for j in 0..6 {
                for p in 0..6 {
                    for q in 0..6 {
                        d[i][j] += r[i][p] * an.d[p][q] * r[j][q];
                    }
                }
            }
        }
        for i in 0..6 {
            for j in 0..i {
                let m = 0.5 * (d[i][j] + d[j][i]);
                d[i][j] = m;
                d[j][i] = m;
            }
        }
        let mut out = Self { alpha: self.alpha, ..Self::anisotropic(d)? };
        if let Some(a) = an.alpha {
            // alpha_g = Q alpha_l Q^T with the same Q as the stiffness (local axes at `phi` to the global ones).
            let (a11, a22, a12) = (a[0], a[1], 0.5 * a[3]);
            let g11 = c * c * a11 + s * s * a22 - 2.0 * c * s * a12;
            let g22 = s * s * a11 + c * c * a22 + 2.0 * c * s * a12;
            let g12 = c * s * (a11 - a22) + (c * c - s * s) * a12;
            // (a_zx, a_zy) rotates as a vector.
            let (zx, zy) = (0.5 * a[5], 0.5 * a[4]);
            let (gzx, gzy) = (c * zx - s * zy, s * zx + c * zy);
            if let Some(o) = out.aniso.as_mut() {
                o.alpha = Some([g11, g22, a[2], 2.0 * g12, 2.0 * gzy, 2.0 * gzx]);
            }
        }
        Ok(out)
    }

    /// Anisotropic thermal expansion: the tensor components `(a_xx, a_yy, a_zz, a_xy, a_yz, a_zx)` per degree (tensor
    /// shear components, in the material axes). Only an anisotropic material has one.
    pub fn with_alpha_tensor(self, a: [f64; 6]) -> Result<Self, String> {
        let mut an = self.aniso.ok_or("a thermal expansion tensor needs an anisotropic material")?;
        an.alpha = Some([a[0], a[1], a[2], 2.0 * a[3], 2.0 * a[4], 2.0 * a[5]]);
        Ok(Self { aniso: Some(an), ..self })
    }

    /// The thermal strain of a uniform temperature change `d_t` as `(xx, yy, zz, 2 xy, 2 yz, 2 zx)`.
    pub fn thermal_strain(&self, d_t: f64) -> [f64; 6] {
        match self.aniso.and_then(|a| a.alpha) {
            Some(a) => a.map(|v| v * d_t),
            None => [self.alpha * d_t, self.alpha * d_t, self.alpha * d_t, 0.0, 0.0, 0.0],
        }
    }

    pub fn with_alpha(self, alpha: f64) -> Self {
        Self { alpha, ..self }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.aniso.is_some() {
            return Ok(()); // positive definiteness was checked on construction
        }
        if !(self.e.is_finite() && self.e > 0.0) {
            return Err("Young's modulus must be positive".into());
        }
        if !(self.nu.is_finite() && self.nu > -1.0 && self.nu < 0.5) {
            return Err("Poisson's ratio must be in (-1, 0.5)".into());
        }
        Ok(())
    }

    /// Shear modulus.
    pub fn mu(&self) -> f64 {
        self.e / (2.0 * (1.0 + self.nu))
    }

    /// First Lame constant for the analysis type (plane stress uses the reduced constant, so the
    /// in-plane law is `sigma = lambda tr(eps) I + 2 mu eps` in every case).
    pub fn lambda(&self, physics: Physics) -> f64 {
        match physics {
            Physics::PlaneStress { .. } => self.e * self.nu / (1.0 - self.nu * self.nu),
            _ => self.e * self.nu / ((1.0 + self.nu) * (1.0 - 2.0 * self.nu)),
        }
    }

    /// Thermal stress per degree of uniform heating when fully constrained: `sigma = -c dT I`.
    pub fn thermal_modulus(&self, physics: Physics) -> f64 {
        let dim_eff = if matches!(physics, Physics::PlaneStress { .. }) { 2.0 } else { 3.0 };
        self.alpha * (dim_eff * self.lambda(physics) + 2.0 * self.mu())
    }
}

/// A group of elements of one type sharing a material.
#[derive(Debug, Clone)]
pub struct Block {
    pub kind: ElementKind,
    /// `n_nodes()` node indices per element, flat.
    pub conn: Vec<usize>,
    pub material: Elastic,
    pub name: String,
    /// J2 plasticity (small or finite strain) for the nonlinear solver; `None` is linear elastic.
    /// The linear solver treats every block as elastic.
    pub plasticity: Option<J2>,
    /// Mass density (mass per volume; plane problems: per area, the thickness is applied by the kernel). `0` = massless:
    /// dynamic analyses (`dynamics.rs`) refuse a mesh without mass.
    pub density: f64,
    /// Conductivity and heat capacity for the heat-conduction analyses (`thermal.rs`).
    pub thermal: Option<crate::thermal::ThermalProps>,
}

impl Block {
    pub fn n_elems(&self) -> usize {
        self.conn.len() / self.kind.n_nodes()
    }

    pub fn elem(&self, e: usize) -> &[usize] {
        let nn = self.kind.n_nodes();
        &self.conn[e * nn..(e + 1) * nn]
    }
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub physics: Physics,
    pub nodes: Vec<[f64; 3]>,
    pub blocks: Vec<Block>,
    pub node_sets: BTreeMap<String, Vec<usize>>,
    /// Named boundary faces (edges in 2D), each a node list ordered counter-clockwise seen from
    /// outside (see `loads.rs`), ready for `Loads::faces`.
    pub surfaces: BTreeMap<String, Vec<Vec<usize>>>,
}

impl Mesh {
    pub fn new(physics: Physics) -> Self {
        Self { physics, nodes: Vec::new(), blocks: Vec::new(), node_sets: BTreeMap::new(), surfaces: BTreeMap::new() }
    }

    pub fn dim(&self) -> usize {
        self.physics.dim()
    }

    pub fn n_dofs(&self) -> usize {
        self.nodes.len() * self.dim()
    }

    pub fn n_elems(&self) -> usize {
        self.blocks.iter().map(Block::n_elems).sum()
    }

    pub fn add_node(&mut self, x: [f64; 3]) -> usize {
        self.nodes.push(x);
        self.nodes.len() - 1
    }

    pub fn add_block(&mut self, kind: ElementKind, conn: Vec<usize>, material: Elastic, name: &str) -> Result<(), String> {
        if kind.dim() != self.dim() {
            return Err(format!("{kind:?} elements do not fit a {}D analysis", self.dim()));
        }
        #[allow(clippy::manual_is_multiple_of)] // is_multiple_of needs a newer toolchain than the CI pin
        if conn.len() % kind.n_nodes() != 0 {
            return Err(format!("connectivity length {} is not a multiple of {}", conn.len(), kind.n_nodes()));
        }
        if let Some(&bad) = conn.iter().find(|&&n| n >= self.nodes.len()) {
            return Err(format!("element refers to node {bad} but the mesh has {}", self.nodes.len()));
        }
        material.validate()?;
        self.blocks.push(Block { kind, conn, material, name: name.to_string(), plasticity: None, density: 0.0, thermal: None });
        Ok(())
    }

    /// Mass density of block `block` (consistent units: with inch / psi use lbf s^2 / in^4, i.e. weight density / 386.09).
    pub fn set_density(&mut self, block: usize, density: f64) -> Result<(), String> {
        if !(density.is_finite() && density > 0.0) {
            return Err(format!("density must be positive and finite, got {density}"));
        }
        self.blocks.get_mut(block).ok_or_else(|| format!("no block {block}"))?.density = density;
        Ok(())
    }

    /// Thermal properties of block `block`.
    pub fn set_thermal(&mut self, block: usize, conductivity: crate::thermal::Conductivity, capacity: f64) -> Result<(), String> {
        conductivity.validate()?;
        if !(capacity.is_finite() && capacity >= 0.0) {
            return Err(format!("heat capacity must be non-negative and finite, got {capacity}"));
        }
        self.blocks.get_mut(block).ok_or_else(|| format!("no block {block}"))?.thermal = Some(crate::thermal::ThermalProps { conductivity, capacity });
        Ok(())
    }

    /// Set the density of every block.
    pub fn set_density_all(&mut self, density: f64) -> Result<(), String> {
        for b in 0..self.blocks.len() {
            self.set_density(b, density)?;
        }
        Ok(())
    }

    /// Attach J2 plasticity to block `block`. An anisotropic elastic law is supported with small-strain plasticity only.
    pub fn set_plasticity(&mut self, block: usize, j2: J2) -> Result<(), String> {
        let b = self.blocks.get_mut(block).ok_or_else(|| format!("no block {block}"))?;
        if b.material.aniso.is_some() && j2.large_strain {
            return Err("finite-strain plasticity needs an isotropic elastic law (anisotropic elasticity is small-strain only)".into());
        }
        if j2.neo_hookean && matches!(self.physics, Physics::PlaneStress { .. }) {
            return Err("neo-Hookean hyperelasticity is not available for plane stress (the out-of-plane stretch would have to be solved for): use plane strain, axisymmetric or solid elements".into());
        }
        b.plasticity = Some(j2);
        Ok(())
    }

    /// Append the nodes, blocks, node sets and surfaces of `other` (same analysis type), prefixing
    /// the names of its sets and surfaces with `prefix` (blocks keep theirs); returns the node index
    /// offset. Bodies stay topologically separate: join them with contact or constraints.
    pub fn append(&mut self, other: &Mesh, prefix: &str) -> Result<usize, String> {
        if std::mem::discriminant(&self.physics) != std::mem::discriminant(&other.physics) {
            return Err("cannot merge meshes of different analysis types".into());
        }
        let off = self.nodes.len();
        self.nodes.extend_from_slice(&other.nodes);
        for b in &other.blocks {
            let mut blk = b.clone();
            blk.conn.iter_mut().for_each(|n| *n += off);
            self.blocks.push(blk);
        }
        for (name, set) in &other.node_sets {
            self.node_sets.insert(format!("{prefix}{name}"), set.iter().map(|n| n + off).collect());
        }
        for (name, faces) in &other.surfaces {
            self.surfaces.insert(format!("{prefix}{name}"), faces.iter().map(|f| f.iter().map(|n| n + off).collect()).collect());
        }
        Ok(off)
    }

    pub fn node_set(&self, name: &str) -> Result<&[usize], String> {
        self.node_sets.get(name).map(Vec::as_slice).ok_or_else(|| format!("no node set '{name}'"))
    }

    /// Nodes satisfying a predicate on their coordinates, as a named set.
    pub fn select_nodes(&mut self, name: &str, pred: impl Fn(&[f64; 3]) -> bool) -> &[usize] {
        let ids: Vec<usize> = (0..self.nodes.len()).filter(|&i| pred(&self.nodes[i])).collect();
        self.node_sets.insert(name.to_string(), ids);
        &self.node_sets[name]
    }
}
