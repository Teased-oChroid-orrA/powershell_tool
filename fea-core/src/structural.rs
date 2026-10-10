//! Small-displacement structural members. Trusses carry axial force; spatial frames use
//! exact two-node Timoshenko bending, Saint-Venant torsion and Euler-Bernoulli consistent
//! uniform-load vectors. No releases, warping, geometric nonlinearity or local buckling.
use crate::fields::{DofMap, Field, FieldAssembly, FieldConstraints, FieldResult};

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f64; 3]) -> Result<[f64; 3], String> {
    let n = a.iter().fold(0.0_f64, |n, v| n.hypot(*v));
    if !n.is_finite() || n <= 0.0 {
        return Err("structural element: degenerate direction".into());
    }
    Ok(a.map(|x| x / n))
}
fn frame(a: [f64; 3], b: [f64; 3], reference: [f64; 3]) -> Result<([[f64; 3]; 3], f64), String> {
    let delta = std::array::from_fn(|i| b[i] - a[i]);
    let l = delta.iter().fold(0.0_f64, |n, v| n.hypot(*v));
    let x = unit(delta)?;
    let z = unit(cross(x, unit(reference)?))?;
    let y = cross(z, x);
    Ok(([x, y, z], l))
}
fn positive(values: &[f64]) -> Result<(), String> {
    if values.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err("structural element: properties must be finite and positive".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct FrameSection {
    pub e: f64,
    pub g: f64,
    pub area: f64,
    pub iy: f64,
    pub iz: f64,
    pub torsion: f64,
    /// Effective shear areas. `None` is Euler-Bernoulli (no shear flexibility).
    pub shear_areas: Option<[f64; 2]>,
}
impl FrameSection {
    fn validate(&self) -> Result<(), String> {
        positive(&[self.e, self.g, self.area, self.iy, self.iz, self.torsion])?;
        if let Some(a) = self.shear_areas {
            positive(&a)?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub enum Member {
    Truss {
        nodes: [usize; 2],
        e: f64,
        area: f64,
    },
    Frame {
        nodes: [usize; 2],
        section: FrameSection,
        reference: [f64; 3],
        uniform_load: [f64; 3],
    },
}
impl Member {
    fn nodes(&self) -> [usize; 2] {
        match self {
            Self::Truss { nodes, .. } | Self::Frame { nodes, .. } => *nodes,
        }
    }
}
#[derive(Debug, Clone)]
pub struct MemberModel {
    pub nodes: Vec<[f64; 3]>,
    pub members: Vec<Member>,
    map: DofMap,
}
#[derive(Debug, Clone)]
pub struct MemberForce {
    /// Local actions of the member on its two ends: [Fx,Fy,Fz,Mx,My,Mz] per end.
    /// Truss moments/shears are zero. Distributed-load vector is subtracted in recovery.
    pub local_end_actions: [f64; 12],
    pub axial_stress: f64,
}
#[derive(Debug, Clone)]
pub struct MemberSolution {
    pub fields: FieldResult,
    pub members: Vec<MemberForce>,
}
struct Element {
    dofs: Vec<usize>,
    k: Vec<f64>,
    local: Vec<f64>,
    transform: Vec<f64>,
    load: Vec<f64>,
    area: f64,
    truss: bool,
}

impl MemberModel {
    pub fn new(nodes: Vec<[f64; 3]>, members: Vec<Member>) -> Result<Self, String> {
        if nodes.is_empty() || members.is_empty() || nodes.iter().flatten().any(|v| !v.is_finite())
        {
            return Err("member model: empty model or nonfinite coordinates".into());
        }
        let mut fields = vec![
            vec![
                Field::Translation(0),
                Field::Translation(1),
                Field::Translation(2)
            ];
            nodes.len()
        ];
        for member in &members {
            let [a, b] = member.nodes();
            if a >= nodes.len() || b >= nodes.len() || a == b {
                return Err("member model: invalid connectivity".into());
            }
            unit(std::array::from_fn(|i| nodes[b][i] - nodes[a][i]))?;
            match member {
                Member::Truss { e, area, .. } => positive(&[*e, *area])?,
                Member::Frame {
                    section,
                    reference,
                    uniform_load,
                    ..
                } => {
                    section.validate()?;
                    frame(nodes[a], nodes[b], *reference)?;
                    if uniform_load.iter().any(|v| !v.is_finite()) {
                        return Err("member model: nonfinite distributed load".into());
                    }
                    for n in [a, b] {
                        if fields[n].len() == 3 {
                            fields[n].extend((0..3).map(Field::Rotation));
                        }
                    }
                }
            }
        }
        let map = DofMap::new(fields)?;
        Ok(Self {
            nodes,
            members,
            map,
        })
    }
    pub fn fields(&self) -> &DofMap {
        &self.map
    }
    fn element(&self, member: &Member) -> Result<Element, String> {
        let [a, b] = member.nodes();
        // Model data are private in the public constructor contract; validate geometry again
        // so changed coordinates cannot produce a silently invalid stiffness.
        let xa = *self.nodes.get(a).ok_or("member: missing node")?;
        let xb = *self.nodes.get(b).ok_or("member: missing node")?;
        match member {
            Member::Truss { e, area, .. } => {
                positive(&[*e, *area])?;
                let delta = std::array::from_fn(|i| xb[i] - xa[i]);
                let direction = unit(delta)?;
                let l = delta.iter().fold(0.0_f64, |s, v| s.hypot(*v));
                let mut k = vec![0.0; 36];
                for i in 0..6 {
                    for j in 0..6 {
                        k[i * 6 + j] = e * area / l
                            * direction[i % 3]
                            * direction[j % 3]
                            * if (i < 3) == (j < 3) { 1.0 } else { -1.0 };
                    }
                }
                let dofs = [a, b]
                    .into_iter()
                    .flat_map(|n| {
                        (0..3).map(move |axis| self.map.index(n, Field::Translation(axis)))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mut transform = vec![0.0; 12];
                for i in 0..3 {
                    transform[i] = direction[i];
                    transform[6 + 3 + i] = direction[i];
                }
                Ok(Element {
                    dofs,
                    k,
                    local: vec![e * area / l, -e * area / l, -e * area / l, e * area / l],
                    transform,
                    load: vec![0.0; 2],
                    area: *area,
                    truss: true,
                })
            }
            Member::Frame {
                section: s,
                reference,
                uniform_load,
                ..
            } => {
                s.validate()?;
                let (axes, l) = frame(xa, xb, *reference)?;
                let mut local = vec![0.0; 144];
                let mut insert = |ids: &[usize], block: &[f64]| {
                    for (i, &a) in ids.iter().enumerate() {
                        for (j, &b) in ids.iter().enumerate() {
                            local[a * 12 + b] += block[i * ids.len() + j];
                        }
                    }
                };
                let axial = s.e * s.area / l;
                let torsion = s.g * s.torsion / l;
                insert(&[0, 6], &[axial, -axial, -axial, axial]);
                insert(&[3, 9], &[torsion, -torsion, -torsion, torsion]);
                for (ids, inertia, shear, sign) in [
                    ([1, 5, 7, 11], s.iz, s.shear_areas.map(|a| a[0]), 1.0),
                    ([2, 4, 8, 10], s.iy, s.shear_areas.map(|a| a[1]), -1.0),
                ] {
                    let phi = shear.map_or(0.0, |area| 12.0 * s.e * inertia / (s.g * area * l * l));
                    let z = s.e * inertia / (l * l * l * (1.0 + phi));
                    let aa = 12.0 * z;
                    let bb = sign * 6.0 * l * z;
                    let cc = (4.0 + phi) * l * l * z;
                    let dd = (2.0 - phi) * l * l * z;
                    insert(
                        &ids,
                        &[
                            aa, bb, -aa, bb, bb, cc, -bb, dd, -aa, -bb, aa, -bb, bb, dd, -bb, cc,
                        ],
                    );
                }
                let mut transform = vec![0.0; 144];
                for base in [0, 3, 6, 9] {
                    for i in 0..3 {
                        for j in 0..3 {
                            transform[(base + i) * 12 + base + j] = axes[i][j];
                        }
                    }
                }
                let mut k = vec![0.0; 144];
                for i in 0..12 {
                    for j in 0..12 {
                        for a in 0..12 {
                            for b in 0..12 {
                                k[i * 12 + j] += transform[a * 12 + i]
                                    * local[a * 12 + b]
                                    * transform[b * 12 + j];
                            }
                        }
                    }
                }
                let q = *uniform_load;
                let mut load = vec![0.0; 12];
                for axis in 0..3 {
                    load[axis] = q[axis] * l / 2.0;
                    load[6 + axis] = load[axis];
                }
                load[5] = q[1] * l * l / 12.0;
                load[11] = -load[5];
                load[4] = -q[2] * l * l / 12.0;
                load[10] = -load[4];
                let dofs = [a, b]
                    .into_iter()
                    .flat_map(|n| {
                        [
                            Field::Translation(0),
                            Field::Translation(1),
                            Field::Translation(2),
                            Field::Rotation(0),
                            Field::Rotation(1),
                            Field::Rotation(2),
                        ]
                        .into_iter()
                        .map(move |f| self.map.index(n, f))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Element {
                    dofs,
                    k,
                    local,
                    transform,
                    load,
                    area: s.area,
                    truss: false,
                })
            }
        }
    }
    pub fn solve(
        &self,
        bc: &FieldConstraints,
        nodal_loads: &[(usize, Field, f64)],
        tol: f64,
    ) -> Result<MemberSolution, String> {
        crate::validate::check_field_supports(
            &self.nodes,
            &self.map,
            &self
                .members
                .iter()
                .map(|m| m.nodes().to_vec())
                .collect::<Vec<_>>(),
            bc,
        )?;
        let mut assembly = FieldAssembly::new(self.map.clone());
        let mut elements = Vec::new();
        for member in &self.members {
            let elem = self.element(member)?;
            assembly.add_element(&elem.dofs, &elem.k)?;
            if !elem.truss {
                for (i, &dof) in elem.dofs.iter().enumerate() {
                    let force = (0..12)
                        .map(|a| elem.transform[a * 12 + i] * elem.load[a])
                        .sum();
                    assembly.add_load(dof, force)?;
                }
            }
            elements.push(elem);
        }
        for &(node, field, force) in nodal_loads {
            assembly.add_load(self.map.index(node, field)?, force)?;
        }
        let fields = assembly.finish().solve(bc, tol)?;
        let mut members = Vec::new();
        for elem in elements {
            let n = elem.dofs.len();
            let m = if elem.truss { 2 } else { 12 };
            let u: Vec<f64> = elem.dofs.iter().map(|&d| fields.values[d]).collect();
            let local_u: Vec<f64> = (0..m)
                .map(|a| (0..n).map(|i| elem.transform[a * n + i] * u[i]).sum())
                .collect();
            let forces: Vec<f64> = (0..m)
                .map(|a| {
                    (0..m)
                        .map(|b| elem.local[a * m + b] * local_u[b])
                        .sum::<f64>()
                        - elem.load[a]
                })
                .collect();
            let mut actions = [0.0; 12];
            if elem.truss {
                actions[0] = forces[0];
                actions[6] = forces[1];
            } else {
                actions.copy_from_slice(&forces);
            }
            if actions.iter().any(|v| !v.is_finite()) {
                return Err("member recovery: nonfinite actions".into());
            }
            members.push(MemberForce {
                local_end_actions: actions,
                axial_stress: actions[6] / elem.area,
            });
        }
        Ok(MemberSolution { fields, members })
    }
}
