//! Authorable aircraft frame / truss member problem over `fea_core::structural`: stable entity
//! ids, atomic edits, validation with explicit refusals, and a solve that only returns results
//! that pass its own acceptance checks (finite values, backward error, force and moment balance).
//! Pure data and functions; the native viewer's editor drives it, nothing here knows about UI.
use fea_core::fields::Field;
use fea_core::structural::{member_axes, FrameSection, Member, MemberModel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const VERSION: u32 = 1;
/// Hard size limits: a typo or a paste must not freeze the editor.
pub const MAX_NODES: usize = 5_000;
pub const MAX_MEMBERS: usize = 10_000;
/// Backward-error and balance thresholds of the acceptance gate.
pub const RESIDUAL_LIMIT: f64 = 1e-8;
pub const BALANCE_LIMIT: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SectionSpec {
    pub e: f64,
    pub g: f64,
    pub area: f64,
    pub iy: f64,
    pub iz: f64,
    pub torsion: f64,
    /// Effective shear areas; `None` is Euler-Bernoulli bending.
    pub shear_areas: Option<[f64; 2]>,
}
impl SectionSpec {
    /// A solid circular section of radius `r` (Timoshenko shear area 0.9 A).
    pub fn solid_round(e: f64, nu: f64, r: f64) -> Self {
        let area = std::f64::consts::PI * r * r;
        let i = std::f64::consts::PI * r.powi(4) / 4.0;
        Self { e, g: e / (2.0 * (1.0 + nu)), area, iy: i, iz: i, torsion: 2.0 * i, shear_areas: Some([0.9 * area; 2]) }
    }
    fn core(&self) -> FrameSection {
        FrameSection { e: self.e, g: self.g, area: self.area, iy: self.iy, iz: self.iz, torsion: self.torsion, shear_areas: self.shear_areas }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MemberKind {
    /// Pin-jointed axial member.
    Truss { e: f64, area: f64 },
    /// Rigid-jointed Timoshenko beam. `uniform_load` is force per length in the member's local axes
    /// (`x` along the member, `z = x × reference`, `y = z × x`; see `fea_core::structural::member_axes`).
    Frame { section: SectionSpec, reference: [f64; 3], uniform_load: [f64; 3] },
}
impl MemberKind {
    pub fn is_frame(&self) -> bool {
        matches!(self, Self::Frame { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: u32,
    pub pos: [f64; 3],
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberSpec {
    pub id: u32,
    pub nodes: [u32; 2],
    pub kind: MemberKind,
}
/// Fixed degrees of freedom `[ux, uy, uz, rx, ry, rz]` (value 0). Rotations need a frame member at the node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupportSpec {
    pub node: u32,
    pub fixed: [bool; 6],
}
/// Applied nodal force and moment (global axes). Moments need a frame member at the node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSpec {
    pub node: u32,
    pub force: [f64; 3],
    pub moment: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberProblem {
    pub version: u32,
    pub name: String,
    /// Display-only unit context ("in, lbf, psi"). The solver is unit-agnostic: inputs must be consistent.
    pub unit_note: String,
    pub nodes: Vec<NodeSpec>,
    pub members: Vec<MemberSpec>,
    pub supports: Vec<SupportSpec>,
    pub loads: Vec<LoadSpec>,
    /// Planar model: the axis (0 x, 1 y, 2 z) normal to the plane of the structure. Every node must lie in
    /// that plane; translation along the axis and (frames) rotations about the in-plane axes are held at
    /// every node, so a flat truss or frame needs only its in-plane supports. `None` = full 3D.
    #[serde(default)]
    pub plane: Option<usize>,
    next_id: u32,
}

/// One atomic edit; `MemberProblem::apply` either performs it entirely or leaves the problem untouched.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    AddNode { pos: [f64; 3] },
    MoveNode { id: u32, pos: [f64; 3] },
    /// Also removes the node's members, support and load.
    RemoveNode { id: u32 },
    AddMember { a: u32, b: u32, kind: MemberKind },
    RemoveMember { id: u32 },
    SetKind { id: u32, kind: MemberKind },
    /// All-false removes the support.
    SetSupport { node: u32, fixed: [bool; 6] },
    /// Zero force and moment remove the load.
    SetLoad { node: u32, force: [f64; 3], moment: [f64; 3] },
    SetUnitNote { note: String },
    /// Switch between planar (`Some(normal axis)`) and 3D (`None`) analysis; planar needs coplanar nodes.
    SetPlane { axis: Option<usize> },
}

impl Default for MemberProblem {
    fn default() -> Self {
        Self::new("untitled")
    }
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}
fn positive(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite() && *x > 0.0)
}

impl MemberProblem {
    pub fn new(name: &str) -> Self {
        Self { version: VERSION, name: name.chars().take(200).collect(), unit_note: String::new(), nodes: vec![], members: vec![], supports: vec![], loads: vec![], plane: None, next_id: 1 }
    }
    pub fn node(&self, id: u32) -> Option<&NodeSpec> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn member(&self, id: u32) -> Option<&MemberSpec> {
        self.members.iter().find(|m| m.id == id)
    }
    pub fn support(&self, node: u32) -> Option<&SupportSpec> {
        self.supports.iter().find(|s| s.node == node)
    }
    pub fn load(&self, node: u32) -> Option<&LoadSpec> {
        self.loads.iter().find(|l| l.node == node)
    }
    /// True when a frame member meets the node (so it carries rotational unknowns).
    pub fn has_frame(&self, node: u32) -> bool {
        self.members.iter().any(|m| m.kind.is_frame() && m.nodes.contains(&node))
    }
    pub fn members_at(&self, node: u32) -> impl Iterator<Item = &MemberSpec> {
        self.members.iter().filter(move |m| m.nodes.contains(&node))
    }
    fn take_id(&mut self) -> Result<u32, String> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or("entity id space exhausted")?;
        Ok(id)
    }

    /// Check one member kind's own properties (without geometry).
    fn check_kind(kind: &MemberKind) -> Result<(), String> {
        match kind {
            MemberKind::Truss { e, area } => {
                if !positive(&[*e, *area]) {
                    return Err("truss: modulus and area must be finite and positive".into());
                }
            }
            MemberKind::Frame { section: s, reference, uniform_load } => {
                if !positive(&[s.e, s.g, s.area, s.iy, s.iz, s.torsion]) || s.shear_areas.is_some_and(|a| !positive(&a)) {
                    return Err("frame: section properties must be finite and positive".into());
                }
                if !finite(reference) || !finite(uniform_load) {
                    return Err("frame: reference direction and distributed load must be finite".into());
                }
            }
        }
        Ok(())
    }

    /// Full consistency check; every refusal says what to change.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err(format!("unsupported member project version {}", self.version));
        }
        if self.nodes.len() > MAX_NODES || self.members.len() > MAX_MEMBERS {
            return Err(format!("model exceeds {MAX_NODES} nodes / {MAX_MEMBERS} members"));
        }
        let mut ids = BTreeMap::new();
        for n in &self.nodes {
            if !finite(&n.pos) {
                return Err(format!("node {} has a nonfinite coordinate", n.id));
            }
            if n.id >= self.next_id || ids.insert(n.id, ()).is_some() {
                return Err(format!("duplicate or invalid node id {}", n.id));
            }
        }
        let mut member_ids = BTreeMap::new();
        let mut pairs = BTreeMap::new();
        for m in &self.members {
            if m.id >= self.next_id || member_ids.insert(m.id, ()).is_some() || ids.contains_key(&m.id) {
                return Err(format!("duplicate or invalid member id {}", m.id));
            }
            let [a, b] = m.nodes;
            let (pa, pb) = (
                self.node(a).ok_or_else(|| format!("member {} references missing node {a}", m.id))?,
                self.node(b).ok_or_else(|| format!("member {} references missing node {b}", m.id))?,
            );
            if a == b {
                return Err(format!("member {} connects node {a} to itself", m.id));
            }
            if pairs.insert((a.min(b), a.max(b)), m.id).is_some() {
                return Err(format!("members {} and {} connect the same nodes", pairs[&(a.min(b), a.max(b))], m.id));
            }
            Self::check_kind(&m.kind)?;
            let delta = [pb.pos[0] - pa.pos[0], pb.pos[1] - pa.pos[1], pb.pos[2] - pa.pos[2]];
            if delta.iter().fold(0.0_f64, |n, v| n.hypot(*v)) <= 0.0 {
                return Err(format!("member {} has zero length", m.id));
            }
            if let MemberKind::Frame { reference, .. } = &m.kind {
                member_axes(pa.pos, pb.pos, *reference)
                    .map_err(|_| format!("member {}: the reference direction must not be parallel to the member", m.id))?;
            }
        }
        if let Some(axis) = self.plane {
            if axis > 2 {
                return Err("plane axis must be 0, 1 or 2".into());
            }
            let name = ["x", "y", "z"][axis];
            let first = self.nodes.first().map_or(0.0, |n| n.pos[axis]);
            let reach = self.nodes.iter().fold(0.0_f64, |a, n| a.max(n.pos.iter().fold(0.0_f64, |m, v| m.max(v.abs())))).max(1.0);
            if self.nodes.iter().any(|n| (n.pos[axis] - first).abs() > 1e-9 * reach) {
                return Err(format!("planar model: every node must lie in one plane {name} = constant"));
            }
            for l in &self.loads {
                let bad_moment = (0..3).any(|k| k != axis && l.moment[k] != 0.0);
                if l.force[axis] != 0.0 || bad_moment {
                    return Err(format!("planar model: node {} has an out-of-plane load (force {name} or moment about an in-plane axis); the constraint would carry it silently", l.node));
                }
            }
        }
        let mut seen = BTreeMap::new();
        for s in &self.supports {
            if self.node(s.node).is_none() || seen.insert(s.node, ()).is_some() {
                return Err(format!("support at missing or repeated node {}", s.node));
            }
            if s.fixed[3..].iter().any(|f| *f) && !self.has_frame(s.node) {
                return Err(format!("node {}: rotations can only be fixed where a frame member is attached", s.node));
            }
        }
        let mut seen = BTreeMap::new();
        for l in &self.loads {
            if self.node(l.node).is_none() || seen.insert(l.node, ()).is_some() {
                return Err(format!("load at missing or repeated node {}", l.node));
            }
            if !finite(&l.force) || !finite(&l.moment) {
                return Err(format!("load at node {} must be finite", l.node));
            }
            if l.moment.iter().any(|m| *m != 0.0) && !self.has_frame(l.node) {
                return Err(format!("node {}: a moment can only be applied where a frame member is attached", l.node));
            }
        }
        Ok(())
    }

    /// Apply an edit atomically. Returns the id of a created node or member.
    pub fn apply(&mut self, edit: Edit) -> Result<Option<u32>, String> {
        let mut next = self.clone();
        let created = next.apply_unchecked(edit)?;
        next.validate()?;
        *self = next;
        Ok(created)
    }
    fn apply_unchecked(&mut self, edit: Edit) -> Result<Option<u32>, String> {
        match edit {
            Edit::AddNode { pos } => {
                if self.nodes.len() >= MAX_NODES {
                    return Err(format!("at most {MAX_NODES} nodes"));
                }
                if self.nodes.iter().any(|n| n.pos == pos) {
                    return Err("a node already exists at that position".into());
                }
                let id = self.take_id()?;
                self.nodes.push(NodeSpec { id, pos });
                Ok(Some(id))
            }
            Edit::MoveNode { id, pos } => {
                if self.nodes.iter().any(|n| n.id != id && n.pos == pos) {
                    return Err("a node already exists at that position".into());
                }
                self.nodes.iter_mut().find(|n| n.id == id).ok_or("no such node")?.pos = pos;
                Ok(None)
            }
            Edit::RemoveNode { id } => {
                if self.node(id).is_none() {
                    return Err("no such node".into());
                }
                self.nodes.retain(|n| n.id != id);
                self.members.retain(|m| !m.nodes.contains(&id));
                self.supports.retain(|s| s.node != id);
                self.loads.retain(|l| l.node != id);
                self.prune_rotational();
                Ok(None)
            }
            Edit::AddMember { a, b, kind } => {
                if self.members.len() >= MAX_MEMBERS {
                    return Err(format!("at most {MAX_MEMBERS} members"));
                }
                let id = self.take_id()?;
                self.members.push(MemberSpec { id, nodes: [a, b], kind });
                Ok(Some(id))
            }
            Edit::RemoveMember { id } => {
                if self.member(id).is_none() {
                    return Err("no such member".into());
                }
                self.members.retain(|m| m.id != id);
                self.prune_rotational();
                Ok(None)
            }
            Edit::SetKind { id, kind } => {
                self.members.iter_mut().find(|m| m.id == id).ok_or("no such member")?.kind = kind;
                self.prune_rotational();
                Ok(None)
            }
            Edit::SetSupport { node, fixed } => {
                self.supports.retain(|s| s.node != node);
                if fixed.iter().any(|f| *f) {
                    self.supports.push(SupportSpec { node, fixed });
                }
                Ok(None)
            }
            Edit::SetLoad { node, force, moment } => {
                self.loads.retain(|l| l.node != node);
                if force.iter().chain(moment.iter()).any(|v| *v != 0.0) {
                    self.loads.push(LoadSpec { node, force, moment });
                }
                Ok(None)
            }
            Edit::SetPlane { axis } => {
                self.plane = axis;
                Ok(None)
            }
            Edit::SetUnitNote { note } => {
                self.unit_note = note.chars().take(100).collect();
                Ok(None)
            }
        }
    }
    /// Removing the last frame member at a node would leave a rotational support or moment on a
    /// translation-only node; drop those so a removal never produces an invalid model.
    fn prune_rotational(&mut self) {
        let frame_nodes: Vec<u32> = self.members.iter().filter(|m| m.kind.is_frame()).flat_map(|m| m.nodes).collect();
        for s in &mut self.supports {
            if !frame_nodes.contains(&s.node) {
                s.fixed[3..].fill(false);
            }
        }
        self.supports.retain(|s| s.fixed.iter().any(|f| *f));
        for l in &mut self.loads {
            if !frame_nodes.contains(&l.node) {
                l.moment = [0.0; 3];
            }
        }
        self.loads.retain(|l| l.force.iter().chain(l.moment.iter()).any(|v| *v != 0.0));
    }

    /// Kernel model plus the id -> node index map (nodes keep their order).
    fn model(&self) -> Result<(MemberModel, BTreeMap<u32, usize>), String> {
        self.validate()?;
        let index: BTreeMap<u32, usize> = self.nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let members = self
            .members
            .iter()
            .map(|m| {
                let nodes = [index[&m.nodes[0]], index[&m.nodes[1]]];
                match m.kind {
                    MemberKind::Truss { e, area } => Member::Truss { nodes, e, area },
                    MemberKind::Frame { section, reference, uniform_load } => Member::Frame { nodes, section: section.core(), reference, uniform_load },
                }
            })
            .collect();
        let model = MemberModel::new(self.nodes.iter().map(|n| n.pos).collect(), members)?;
        Ok((model, index))
    }

    /// Solve and accept: a result is returned only when every check in [`Acceptance`] passes.
    pub fn solve(&self) -> Result<MemberSolved, String> {
        let (model, index) = self.model()?;
        let map = model.fields();
        let mut bc = map.constraints();
        for s in &self.supports {
            for (k, fixed) in s.fixed.iter().enumerate() {
                if *fixed {
                    let field = if k < 3 { Field::Translation(k) } else { Field::Rotation(k - 3) };
                    bc.prescribe(map, index[&s.node], field, 0.0)?;
                }
            }
        }
        if let Some(axis) = self.plane {
            for (i, _) in self.nodes.iter().enumerate() {
                bc.prescribe(map, i, Field::Translation(axis), 0.0)?;
                if map.index(i, Field::Rotation(0)).is_ok() {
                    for k in (0..3).filter(|k| *k != axis) {
                        bc.prescribe(map, i, Field::Rotation(k), 0.0)?;
                    }
                }
            }
        }
        let mut loads = Vec::new();
        for l in &self.loads {
            for k in 0..3 {
                if l.force[k] != 0.0 {
                    loads.push((index[&l.node], Field::Translation(k), l.force[k]));
                }
                if l.moment[k] != 0.0 {
                    loads.push((index[&l.node], Field::Rotation(k), l.moment[k]));
                }
            }
        }
        let solution = model.solve(&bc, &loads, 1e-12)?;
        let n = self.nodes.len();
        let mut displacement = vec![[0.0; 6]; n];
        let mut reaction = vec![[0.0; 6]; n];
        for i in 0..n {
            for k in 0..6 {
                let field = if k < 3 { Field::Translation(k) } else { Field::Rotation(k - 3) };
                if let Ok(dof) = map.index(i, field) {
                    displacement[i][k] = solution.fields.values[dof];
                    reaction[i][k] = solution.fields.reactions[dof];
                }
            }
        }
        let mut members = Vec::new();
        let mut applied_force = [0.0; 3];
        let mut applied_moment = [0.0; 3];
        for l in &self.loads {
            let p = self.node(l.node).expect("validated").pos;
            let m = cross(p, l.force);
            for k in 0..3 {
                applied_force[k] += l.force[k];
                applied_moment[k] += l.moment[k] + m[k];
            }
        }
        for (spec, force) in self.members.iter().zip(&solution.members) {
            let a = self.node(spec.nodes[0]).expect("validated").pos;
            let b = self.node(spec.nodes[1]).expect("validated").pos;
            let length = (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f64>().sqrt();
            let act = &force.local_end_actions;
            let mut max_moment = 0.0_f64;
            if let MemberKind::Frame { reference, uniform_load, .. } = spec.kind {
                for end in [3, 9] {
                    max_moment = max_moment.max(act[end + 1].hypot(act[end + 2]));
                }
                // Resultant of the distributed load, global components, acting at the midpoint.
                let (axes, l) = member_axes(a, b, reference)?;
                let total: [f64; 3] = std::array::from_fn(|g| (0..3).map(|k| axes[k][g] * uniform_load[k] * l).sum());
                let mid = std::array::from_fn(|k| 0.5 * (a[k] + b[k]));
                let m = cross(mid, total);
                for k in 0..3 {
                    applied_force[k] += total[k];
                    applied_moment[k] += m[k];
                }
            }
            members.push(MemberResult { id: spec.id, length, axial_force: act[6], axial_stress: force.axial_stress, max_bending_moment: max_moment, end_actions: *act });
        }
        // Force / moment balance of applied loads against the support reactions (moments about the origin).
        let mut sum_f = applied_force;
        let mut sum_m = applied_moment;
        for (node, r) in self.nodes.iter().zip(&reaction) {
            let m = cross(node.pos, [r[0], r[1], r[2]]);
            for k in 0..3 {
                sum_f[k] += r[k];
                sum_m[k] += r[3 + k] + m[k];
            }
        }
        let norm = |v: [f64; 3]| v.iter().fold(0.0_f64, |a, x| a.hypot(*x));
        let reaction_force = reaction.iter().fold(0.0_f64, |a, r| a.hypot(norm([r[0], r[1], r[2]])));
        let force_scale = norm(applied_force).max(reaction_force).max(f64::MIN_POSITIVE);
        let reach = self.nodes.iter().fold(0.0_f64, |a, n| a.max(norm(n.pos)));
        let moment_scale = norm(applied_moment)
            .max(reaction.iter().fold(0.0_f64, |a, r| a.hypot(norm([r[3], r[4], r[5]]))))
            .max(force_scale * reach)
            .max(f64::MIN_POSITIVE);
        let acceptance = Acceptance {
            rel_residual: solution.fields.rel_residual,
            force_balance: norm(sum_f) / force_scale,
            moment_balance: norm(sum_m) / moment_scale,
        };
        let all_finite = displacement.iter().chain(&reaction).flatten().all(|v| v.is_finite())
            && members.iter().all(|m| m.axial_force.is_finite() && m.max_bending_moment.is_finite());
        acceptance.check(all_finite)?;
        Ok(MemberSolved { displacement, reaction, members, acceptance, strain_energy: 0.5 * solution.fields.quadratic_energy })
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Acceptance {
    /// Backward error of the linear solve on the free DOFs.
    pub rel_residual: f64,
    /// |sum of applied + reaction forces| / force scale.
    pub force_balance: f64,
    /// |sum of applied + reaction moments about the origin| / moment scale.
    pub moment_balance: f64,
}
impl Acceptance {
    fn check(&self, finite: bool) -> Result<(), String> {
        if !finite {
            return Err("rejected: nonfinite displacement, reaction or member action".into());
        }
        if self.rel_residual.is_nan() || self.rel_residual > RESIDUAL_LIMIT {
            return Err(format!("rejected: solve residual {:.2e} exceeds {RESIDUAL_LIMIT:.0e} (nearly singular model?)", self.rel_residual));
        }
        let balanced = |v: f64| v <= BALANCE_LIMIT; // false for NaN
        if !(balanced(self.force_balance) && balanced(self.moment_balance)) {
            return Err(format!("rejected: equilibrium error force {:.2e}, moment {:.2e} exceeds {BALANCE_LIMIT:.0e}", self.force_balance, self.moment_balance));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberResult {
    pub id: u32,
    pub length: f64,
    /// Tension positive: the local axial action at the far end.
    pub axial_force: f64,
    pub axial_stress: f64,
    /// Largest end moment magnitude about the local y/z axes (frames; 0 for trusses). Uniform-load
    /// spans can exceed their end values; this is an end-action, not a span envelope.
    pub max_bending_moment: f64,
    pub end_actions: [f64; 12],
}
#[derive(Debug, Clone)]
pub struct MemberSolved {
    /// Per node (input order): `[ux, uy, uz, rx, ry, rz]`; rotations are 0 where the node has none.
    pub displacement: Vec<[f64; 6]>,
    /// Support reactions, same layout; the supports' action on the structure.
    pub reaction: Vec<[f64; 6]>,
    pub members: Vec<MemberResult>,
    pub acceptance: Acceptance,
    pub strain_energy: f64,
}
impl MemberSolved {
    pub fn max_displacement(&self) -> f64 {
        self.displacement.iter().fold(0.0_f64, |a, d| a.max((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()))
    }
}

/// Starting problems for the editor.
pub fn templates() -> Vec<MemberProblem> {
    vec![tripod(), cantilever_frame()]
}
/// An empty planar model (plane z = 0): the natural start for drawing a flat truss or frame.
pub fn blank_planar() -> MemberProblem {
    let mut p = MemberProblem::new("New planar model");
    p.unit_note = "in, lbf, psi".into();
    p.plane = Some(2);
    p
}
fn tripod() -> MemberProblem {
    let mut p = MemberProblem::new("Truss tripod");
    p.unit_note = "in, lbf, psi".into();
    let mut base = Vec::new();
    for i in 0..3 {
        let a = 2.0 * std::f64::consts::PI * i as f64 / 3.0;
        let id = p.apply(Edit::AddNode { pos: [20.0 * a.cos(), 20.0 * a.sin(), 0.0] }).unwrap().unwrap();
        p.apply(Edit::SetSupport { node: id, fixed: [true, true, true, false, false, false] }).unwrap();
        base.push(id);
    }
    let apex = p.apply(Edit::AddNode { pos: [0.0, 0.0, 15.0] }).unwrap().unwrap();
    for b in base {
        p.apply(Edit::AddMember { a: b, b: apex, kind: MemberKind::Truss { e: 1.0e7, area: 0.5 } }).unwrap();
    }
    p.apply(Edit::SetLoad { node: apex, force: [0.0, 0.0, -1000.0], moment: [0.0; 3] }).unwrap();
    p
}
fn cantilever_frame() -> MemberProblem {
    let mut p = MemberProblem::new("Cantilever frame");
    p.unit_note = "in, lbf, psi".into();
    let section = SectionSpec::solid_round(1.0e7, 0.33, 1.0);
    let a = p.apply(Edit::AddNode { pos: [0.0; 3] }).unwrap().unwrap();
    let b = p.apply(Edit::AddNode { pos: [50.0, 0.0, 0.0] }).unwrap().unwrap();
    p.apply(Edit::AddMember { a, b, kind: MemberKind::Frame { section, reference: [0.0, 1.0, 0.0], uniform_load: [0.0; 3] } }).unwrap();
    p.apply(Edit::SetSupport { node: a, fixed: [true; 6] }).unwrap();
    p.apply(Edit::SetLoad { node: b, force: [0.0, 100.0, 0.0], moment: [0.0; 3] }).unwrap();
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acceptance_gate_rejects_each_failure_kind() {
        let ok = Acceptance { rel_residual: 1e-13, force_balance: 1e-12, moment_balance: 1e-12 };
        assert!(ok.check(true).is_ok());
        assert!(ok.check(false).unwrap_err().contains("nonfinite"));
        assert!(Acceptance { rel_residual: 1e-5, ..ok }.check(true).unwrap_err().contains("residual"));
        assert!(Acceptance { rel_residual: f64::NAN, ..ok }.check(true).is_err());
        assert!(Acceptance { force_balance: 1e-3, ..ok }.check(true).unwrap_err().contains("equilibrium"));
        assert!(Acceptance { moment_balance: f64::NAN, ..ok }.check(true).is_err());
    }
}
