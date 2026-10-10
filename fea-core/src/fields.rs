//! Generalized, node-major fields and symmetric assembly. Continuum layouts are preserved;
//! structural rotations and thermal fields can be present only where an element needs them.
use crate::assembly::{BlockMatrix, Pattern};
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::Model;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Translation(usize),
    Rotation(usize),
    Temperature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DofMap {
    nodes: Vec<Vec<Field>>,
    offsets: Vec<usize>,
}
impl DofMap {
    pub fn new(nodes: Vec<Vec<Field>>) -> Result<Self, String> {
        let mut offsets = vec![0usize];
        for fields in &nodes {
            if fields.is_empty()
                || fields
                    .iter()
                    .any(|f| matches!(f, Field::Translation(a) | Field::Rotation(a) if *a >= 3))
            {
                return Err("DOF map: empty node fields or axis outside 0..3".into());
            }
            for (i, field) in fields.iter().enumerate() {
                if fields[..i].contains(field) {
                    return Err("DOF map: duplicate field on node".into());
                }
            }
            let next = offsets
                .last()
                .unwrap()
                .checked_add(fields.len())
                .ok_or("DOF map overflow")?;
            if next > u32::MAX as usize {
                return Err("DOF map exceeds sparse index capacity".into());
            }
            offsets.push(next);
        }
        if nodes.is_empty() {
            return Err("DOF map: no nodes".into());
        }
        Ok(Self { nodes, offsets })
    }
    pub fn uniform(n: usize, fields: &[Field]) -> Result<Self, String> {
        Self::new(vec![fields.to_vec(); n])
    }
    pub fn n_dofs(&self) -> usize {
        *self.offsets.last().unwrap()
    }
    pub fn n_nodes(&self) -> usize {
        self.nodes.len()
    }
    pub fn index(&self, node: usize, field: Field) -> Result<usize, String> {
        let fields = self.nodes.get(node).ok_or("DOF map: missing node")?;
        fields
            .iter()
            .position(|f| *f == field)
            .map(|i| self.offsets[node] + i)
            .ok_or_else(|| "DOF map: missing field".into())
    }
    pub fn fields(&self, node: usize) -> Option<&[Field]> {
        self.nodes.get(node).map(Vec::as_slice)
    }
    pub fn constraints(&self) -> FieldConstraints {
        FieldConstraints {
            values: vec![None; self.n_dofs()],
        }
    }
}

#[derive(Debug, Clone)]
pub struct FieldConstraints {
    values: Vec<Option<f64>>,
}
impl FieldConstraints {
    pub fn prescribed(&self, dof: usize) -> Result<Option<f64>, String> {
        self.values
            .get(dof)
            .copied()
            .ok_or_else(|| "field constraint: DOF out of range".into())
    }
    pub fn prescribe(
        &mut self,
        map: &DofMap,
        node: usize,
        field: Field,
        value: f64,
    ) -> Result<(), String> {
        if !value.is_finite() || self.values.len() != map.n_dofs() {
            return Err("field constraint: nonfinite value or layout mismatch".into());
        }
        let dof = map.index(node, field)?;
        if let Some(old) = self.values[dof] {
            if old != value {
                return Err("field constraint: conflicting prescription".into());
            }
        }
        self.values[dof] = Some(value);
        Ok(())
    }
    pub fn from_continuum(model: &Model, bc: &Dirichlet) -> Result<Self, String> {
        if bc.d != model.mesh.dim()
            || bc.fixed.len() != model.mesh.n_dofs()
            || bc.value.len() != bc.fixed.len()
            || bc.value.iter().any(|v| !v.is_finite())
        {
            return Err("field constraint: invalid continuum layout".into());
        }
        Ok(Self {
            values: bc
                .fixed
                .iter()
                .zip(&bc.value)
                .map(|(&fixed, &v)| fixed.then_some(v))
                .collect(),
        })
    }
}

/// Element matrices are row-major and symmetric. Assembly is deterministic and stores only
/// the sparse lower triangle; it does not allocate a dense global matrix.
pub struct FieldAssembly {
    map: DofMap,
    entries: BTreeMap<(usize, usize), f64>,
    load: Vec<f64>,
}
impl FieldAssembly {
    pub fn new(map: DofMap) -> Self {
        let load = vec![0.0; map.n_dofs()];
        Self {
            map,
            entries: BTreeMap::new(),
            load,
        }
    }
    pub fn add_load(&mut self, dof: usize, force: f64) -> Result<(), String> {
        if dof >= self.load.len() || !force.is_finite() || !(self.load[dof] + force).is_finite() {
            return Err("field load: invalid DOF or nonfinite force".into());
        }
        self.load[dof] += force;
        Ok(())
    }
    pub fn add_element(&mut self, dofs: &[usize], matrix: &[f64]) -> Result<(), String> {
        let n = dofs.len();
        if n == 0
            || n.checked_mul(n) != Some(matrix.len())
            || dofs.iter().any(|&d| d >= self.map.n_dofs())
            || matrix.iter().any(|x| !x.is_finite())
        {
            return Err("field assembly: invalid element dimensions, DOFs or values".into());
        }
        for (i, d) in dofs.iter().enumerate() {
            if dofs[..i].contains(d) {
                return Err("field assembly: repeated element DOF".into());
            }
        }
        let scale = matrix.iter().fold(0.0_f64, |s, v| s.max(v.abs()));
        for i in 0..n {
            for j in 0..i {
                if (matrix[i * n + j] - matrix[j * n + i]).abs() > 1e-12 * scale {
                    return Err("field assembly: nonsymmetric element matrix".into());
                }
            }
        }
        // Validate every sum before mutating, so a rejected element leaves assembly unchanged.
        let mut additions = Vec::new();
        for i in 0..n {
            for j in 0..=i {
                let key = (dofs[i].min(dofs[j]), dofs[i].max(dofs[j]));
                let value = self.entries.get(&key).copied().unwrap_or(0.0) + matrix[i * n + j];
                if !value.is_finite() {
                    return Err("field assembly overflow".into());
                }
                additions.push((key, value));
            }
        }
        self.entries.extend(additions);
        Ok(())
    }
    pub fn finish(mut self) -> FieldSystem {
        let n = self.map.n_dofs();
        for i in 0..n {
            self.entries.entry((i, i)).or_insert(0.0);
        }
        let mut ptr = vec![0; n + 1];
        let mut row = Vec::new();
        let mut vals = Vec::new();
        for ((col, r), v) in self.entries {
            ptr[col + 1] += 1;
            row.push(r as u32);
            vals.push(v);
        }
        for i in 0..n {
            ptr[i + 1] += ptr[i];
        }
        let pattern = Arc::new(Pattern {
            n_nodes: n,
            d: 1,
            col_ptr: ptr,
            row_idx: row,
            scatter: Vec::new(),
            colors: Vec::new(),
        });
        FieldSystem {
            map: self.map,
            pattern,
            matrix: BlockMatrix { d: 1, vals },
            load: self.load,
        }
    }
}

pub struct FieldSystem {
    pub map: DofMap,
    pub pattern: Arc<Pattern>,
    pub matrix: BlockMatrix,
    pub load: Vec<f64>,
}
#[derive(Debug, Clone)]
pub struct FieldResult {
    pub map: DofMap,
    pub values: Vec<f64>,
    pub reactions: Vec<f64>,
    pub rel_residual: f64,
    pub quadratic_energy: f64,
}
impl FieldResult {
    pub fn value(&self, node: usize, field: Field) -> Result<f64, String> {
        Ok(self.values[self.map.index(node, field)?])
    }
}
impl FieldSystem {
    pub fn solve(&self, constraints: &FieldConstraints, tol: f64) -> Result<FieldResult, String> {
        let n = self.map.n_dofs();
        if !tol.is_finite()
            || tol <= 0.0
            || constraints.values.len() != n
            || self.load.len() != n
            || self
                .load
                .iter()
                .chain(&self.matrix.vals)
                .any(|v| !v.is_finite())
        {
            return Err("field solve: invalid tolerance, dimensions or values".into());
        }
        let mut bc = Dirichlet::new(n, 1);
        for (i, &v) in constraints.values.iter().enumerate() {
            if let Some(v) = v {
                bc.fix(i, 0, v);
            }
        }
        let values = if bc.n_fixed() == n {
            bc.value.clone()
        } else {
            let red = Reduced::with_ordering(&self.pattern, &bc, None, Ordering::Amd)
                .map_err(|e| e.to_string())?;
            let factor = red.factor(&self.matrix).map_err(|e| e.to_string())?;
            factor.solve(&self.matrix, &self.load, &bc)
        };
        let mut ku = vec![0.0; n];
        self.matrix.matvec_add(&self.pattern, &values, &mut ku);
        let abs = BlockMatrix {
            d: 1,
            vals: self.matrix.vals.iter().map(|v| v.abs()).collect(),
        };
        let mut scale = vec![0.0; n];
        abs.matvec_add(
            &self.pattern,
            &values.iter().map(|v| v.abs()).collect::<Vec<_>>(),
            &mut scale,
        );
        let mut rn = 0.0_f64;
        let mut sn = 0.0_f64;
        for i in 0..n {
            if !bc.fixed[i] {
                rn = rn.hypot(ku[i] - self.load[i]);
                sn = sn.hypot(scale[i] + self.load[i].abs());
            }
        }
        let rel_residual = rn / sn.max(1e-300);
        let energy = 0.5 * values.iter().zip(&ku).map(|(u, f)| u * f).sum::<f64>();
        if values.iter().chain(&ku).any(|v| !v.is_finite())
            || !sn.is_finite()
            || !rel_residual.is_finite()
            || rel_residual > tol
            || !energy.is_finite()
        {
            return Err(format!("field solve: failed finite-state/residual acceptance ({rel_residual:e}, allowed {tol:e})"));
        }
        let reactions = ku.iter().zip(&self.load).map(|(k, f)| k - f).collect();
        Ok(FieldResult {
            map: self.map.clone(),
            values,
            reactions,
            rel_residual,
            quadratic_energy: energy,
        })
    }
}
impl Model {
    pub fn displacement_fields(&self) -> DofMap {
        DofMap::uniform(
            self.mesh.nodes.len(),
            &(0..self.mesh.dim())
                .map(Field::Translation)
                .collect::<Vec<_>>(),
        )
        .expect("valid continuum layout")
    }
    pub fn temperature_fields(&self) -> DofMap {
        DofMap::uniform(self.mesh.nodes.len(), &[Field::Temperature])
            .expect("valid continuum layout")
    }
}
