//! Editor state machine: tool semantics, selection, undo/redo, property fields and persistence.
//! Everything here is a pure function of the problem and the input events it is given.
use super::camera::{snap, Frame, Hit};
use fea_problem::members::{Edit, MemberKind, MemberProblem, MemberSolved, SectionSpec};
use std::sync::Arc;

pub const UNDO_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tool {
    Select,
    Node,
    Truss,
    Frame,
    Support,
    Load,
}
impl Tool {
    pub const ALL: [Tool; 6] = [Tool::Select, Tool::Node, Tool::Truss, Tool::Frame, Tool::Support, Tool::Load];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Node => "Add node",
            Tool::Truss => "Add truss member",
            Tool::Frame => "Add frame member",
            Tool::Support => "Add support",
            Tool::Load => "Add load",
        }
    }
    pub fn key(self) -> char {
        match self {
            Tool::Select => '1',
            Tool::Node => '2',
            Tool::Truss => '3',
            Tool::Frame => '4',
            Tool::Support => '5',
            Tool::Load => '6',
        }
    }
}

/// Boundary conditions the Support tool offers for the picked node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportPreset {
    /// Every translation (and rotation at a frame node) held.
    Fixed,
    /// Translations held, rotations free.
    Pinned,
    /// Free to slide along one axis (0 = x, 1 = y, 2 = z); the other translations are held.
    Roller(usize),
    Remove,
}
impl SupportPreset {
    pub const ALL: [SupportPreset; 6] = [Self::Fixed, Self::Pinned, Self::Roller(0), Self::Roller(1), Self::Roller(2), Self::Remove];
    pub fn label(self) -> String {
        match self {
            Self::Fixed => "Fixed (all motion held)".into(),
            Self::Pinned => "Pinned (translations held, rotation free)".into(),
            Self::Roller(a) => format!("Roller (free to slide in {})", ["x", "y", "z"][a]),
            Self::Remove => "Remove support".into(),
        }
    }
    /// Held degrees of freedom for a node (`frame`: it has rotations).
    pub fn fixed(self, frame: bool) -> [bool; 6] {
        match self {
            Self::Fixed => if frame { [true; 6] } else { [true, true, true, false, false, false] },
            Self::Pinned => [true, true, true, false, false, false],
            Self::Roller(a) => std::array::from_fn(|k| k < 3 && k != a),
            Self::Remove => [false; 6],
        }
    }
}

/// Defaults used when a tool creates something; editable through the property rows of the result.
#[derive(Clone, Debug, PartialEq)]
pub struct Defaults {
    pub truss: (f64, f64),
    pub section: SectionSpec,
    pub reference: [f64; 3],
    pub force: [f64; 3],
}
impl Default for Defaults {
    fn default() -> Self {
        Self { truss: (1.0e7, 0.5), section: SectionSpec::solid_round(1.0e7, 0.33, 1.0), reference: [0.0, 0.0, 1.0], force: [0.0, 0.0, -100.0] }
    }
}

/// One editable number of the selection.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub label: String,
    pub value: f64,
    pub kind: PropertyKind,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropertyKind {
    /// Distance to move every selected node along an axis.
    MoveGroup(usize),
    /// Offset of the working plane along its normal (where new nodes are placed).
    PlaneOffset,
    Coordinate(usize),
    Force(usize),
    Moment(usize),
    /// Fixed (1) or free (0) degree of freedom 0..6.
    Fixed(usize),
    TrussE,
    TrussArea,
    SecE,
    SecG,
    SecArea,
    SecIy,
    SecIz,
    SecTorsion,
    /// Uniform load per length, local axis.
    Distributed(usize),
}

#[derive(Clone, Debug)]
pub struct Solution {
    /// The exact problem this result belongs to; any difference makes it stale.
    pub for_problem: MemberProblem,
    pub solved: Arc<MemberSolved>,
}

#[derive(Debug)]
pub struct EditorState {
    pub problem: MemberProblem,
    undo: Vec<MemberProblem>,
    redo: Vec<MemberProblem>,
    pub selection: Option<Hit>,
    pub tool: Tool,
    /// First node of a member being created.
    pub pending: Option<u32>,
    pub plane_axis: usize,
    pub plane_value: f64,
    pub grid: f64,
    pub frame: Frame,
    pub defaults: Defaults,
    pub solution: Option<Solution>,
    /// Last explanation or error, shown in the HUD.
    pub status: String,
    /// Index of the focused property row.
    pub focus: usize,
    /// True when the problem differs from what was last saved/loaded.
    pub dirty: bool,
    saved: Option<MemberProblem>,
    /// A Support / Load click on a node asks for the list of applicable items for that node (taken by the session).
    pub popup: Option<(Tool, u32)>,
    /// Further selected entities (Shift-click, box select); `selection` is the last picked one.
    pub group: Vec<Hit>,
    pub show_supports: bool,
    pub show_loads: bool,
}

impl EditorState {
    pub fn new(problem: MemberProblem) -> Self {
        let frame = Frame::fit(&problem);
        Self {
            saved: Some(problem.clone()),
            problem,
            undo: vec![],
            redo: vec![],
            selection: None,
            tool: Tool::Select,
            pending: None,
            plane_axis: 2,
            plane_value: 0.0,
            grid: 5.0,
            frame,
            defaults: Defaults::default(),
            solution: None,
            status: String::new(),
            focus: 0,
            dirty: false,
            popup: None,
            group: Vec::new(),
            show_supports: true,
            show_loads: true,
        }
    }

    /// Result for the current problem, or `None` when absent or stale.
    pub fn current_solution(&self) -> Option<&Arc<MemberSolved>> {
        self.solution.as_ref().filter(|s| s.for_problem == self.problem).map(|s| &s.solved)
    }
    pub fn solution_is_stale(&self) -> bool {
        self.solution.as_ref().is_some_and(|s| s.for_problem != self.problem)
    }
    pub fn accept_solution(&mut self, solved: MemberSolved, for_problem: MemberProblem) {
        self.solution = Some(Solution { for_problem, solved: Arc::new(solved) });
    }

    fn mark(&mut self) {
        self.group.retain(|h| match h {
            Hit::Node(id) => self.problem.node(*id).is_some(),
            Hit::Member(id) => self.problem.member(*id).is_some(),
        });
        self.dirty = self.saved.as_ref() != Some(&self.problem);
        // Keep the selection valid after structural edits.
        self.selection = self.selection.filter(|h| match h {
            Hit::Node(id) => self.problem.node(*id).is_some(),
            Hit::Member(id) => self.problem.member(*id).is_some(),
        });
        if self.pending.is_some_and(|n| self.problem.node(n).is_none()) {
            self.pending = None;
        }
        self.focus = self.focus.min(self.properties().len().saturating_sub(1));
    }

    /// Apply one edit; on success it becomes undoable and clears the redo history.
    pub fn commit(&mut self, edit: Edit) -> Result<Option<u32>, String> {
        let before = self.problem.clone();
        match self.problem.apply(edit) {
            Ok(id) => {
                if before != self.problem {
                    self.undo.push(before);
                    if self.undo.len() > UNDO_LIMIT {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                }
                self.status.clear();
                self.mark();
                Ok(id)
            }
            Err(e) => {
                self.status = e.clone();
                Err(e)
            }
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.problem, prev));
        self.pending = None;
        self.mark();
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.problem, next));
        self.pending = None;
        self.mark();
        true
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.pending = None;
        self.status.clear();
    }
    /// Every selected entity, the last picked one first.
    pub fn selected_all(&self) -> Vec<Hit> {
        self.selection.into_iter().chain(self.group.iter().copied().filter(|h| Some(*h) != self.selection)).collect()
    }
    /// Shift-click: add the entity to the selection, or drop it when it is already selected.
    pub fn toggle(&mut self, hit: Hit) {
        let mut all = self.selected_all();
        if let Some(i) = all.iter().position(|h| *h == hit) {
            all.remove(i);
        } else {
            all.insert(0, hit);
        }
        self.selection = all.first().copied();
        self.group = all.into_iter().skip(1).collect();
        self.focus = 0;
    }
    /// Replace (or with `additive` extend) the selection by `hits`.
    pub fn select_many(&mut self, hits: Vec<Hit>, additive: bool) {
        let mut all = if additive { self.selected_all() } else { Vec::new() };
        for h in hits {
            if !all.contains(&h) {
                all.insert(0, h);
            }
        }
        self.selection = all.first().copied();
        self.group = all.into_iter().skip(1).collect();
        self.focus = 0;
    }
    /// Apply `edits` in order as ONE undo step; all-or-nothing.
    pub fn commit_all(&mut self, edits: Vec<Edit>) -> Result<(), String> {
        let before = self.problem.clone();
        for e in edits {
            if let Err(err) = self.problem.apply(e) {
                self.problem = before;
                self.status = err.clone();
                return Err(err);
            }
        }
        if before != self.problem {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.mark();
        Ok(())
    }
    /// Nodes a group operation moves: selected nodes and the ends of selected members.
    pub fn group_nodes(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = Vec::new();
        for h in self.selected_all() {
            match h {
                Hit::Node(n) => ids.push(n),
                Hit::Member(m) => {
                    if let Some(spec) = self.problem.member(m) {
                        ids.extend(spec.nodes);
                    }
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
    /// Move the group by `delta` along `axis` (one undo step).
    pub fn move_group(&mut self, axis: usize, delta: f64) -> Result<(), String> {
        if !delta.is_finite() {
            return Err("distance must be a finite number".into());
        }
        let ids = self.group_nodes();
        if ids.is_empty() {
            return Err("select nodes or members first".into());
        }
        let edits = ids
            .iter()
            .filter_map(|id| {
                let mut pos = self.problem.node(*id)?.pos;
                pos[axis] += delta;
                Some(Edit::MoveNode { id: *id, pos })
            })
            .collect();
        self.commit_all(edits)
    }
    pub fn select(&mut self, hit: Option<Hit>) {
        self.group.clear();
        self.selection = hit;
        self.focus = 0;
        if let Some(Hit::Node(id)) = hit {
            if let Some(n) = self.problem.node(id) {
                self.plane_value = n.pos[self.plane_axis];
            }
        }
    }
    pub fn cycle_plane(&mut self) {
        self.plane_axis = (self.plane_axis + 1) % 3;
        self.plane_value = match self.selection {
            Some(Hit::Node(id)) => self.problem.node(id).map_or(0.0, |n| n.pos[self.plane_axis]),
            _ => 0.0,
        };
    }
    pub fn plane_name(&self) -> &'static str {
        ["x", "y", "z"][self.plane_axis]
    }

    /// Interpret a click. `hit` is the picked entity; `plane` the cursor on the working plane in
    /// model coordinates (`None` when the plane is edge-on). Errors are also stored in `status`.
    pub fn click(&mut self, hit: Option<Hit>, plane: Option<[f64; 3]>) -> Result<(), String> {
        let place = |s: &Self| -> Option<[f64; 3]> {
            plane.map(|p| {
                let mut q = p.map(|v| snap(v, s.grid));
                q[s.plane_axis] = s.plane_value;
                q
            })
        };
        let result = match self.tool {
            Tool::Select => {
                self.select(hit);
                Ok(())
            }
            Tool::Node => match hit {
                Some(h) => {
                    self.select(Some(h));
                    Ok(())
                }
                None => match place(self) {
                    Some(pos) => self.commit(Edit::AddNode { pos }).map(|id| self.select(id.map(Hit::Node))),
                    None => Err("this view is edge-on to the working plane: orbit the camera or press P to change the plane".to_string()),
                },
            },
            Tool::Truss | Tool::Frame => {
                let node = match hit {
                    Some(Hit::Node(id)) => Ok(id),
                    Some(Hit::Member(_)) => Err("click a node (or empty space to create one) to connect members".to_string()),
                    None => match place(self) {
                        Some(pos) => self.commit(Edit::AddNode { pos }).map(|id| id.expect("AddNode returns an id")),
                        None => Err("this view is edge-on to the working plane: orbit the camera or press P to change the plane".to_string()),
                    },
                };
                node.and_then(|n| match self.pending.take() {
                    None => {
                        self.pending = Some(n);
                        self.select(Some(Hit::Node(n)));
                        Ok(())
                    }
                    Some(a) if a == n => Ok(()),
                    Some(a) => {
                        let kind = if self.tool == Tool::Truss {
                            MemberKind::Truss { e: self.defaults.truss.0, area: self.defaults.truss.1 }
                        } else {
                            MemberKind::Frame { section: self.defaults.section, reference: self.default_reference(a, n), uniform_load: [0.0; 3] }
                        };
                        let r = self.commit(Edit::AddMember { a, b: n, kind });
                        if let Ok(id) = &r {
                            self.select(id.map(Hit::Member));
                        } else {
                            self.pending = Some(a);
                        }
                        r.map(|_| ())
                    }
                })
            }
            // Boundary conditions are chosen from a list of what applies to the clicked node.
            Tool::Support | Tool::Load => match hit {
                Some(Hit::Node(id)) => {
                    self.select(Some(Hit::Node(id)));
                    self.popup = Some((self.tool, id));
                    Ok(())
                }
                _ => Err(format!("click a node to {}", if self.tool == Tool::Support { "choose its support" } else { "apply or edit its load" })),
            },
        };
        if let Err(e) = &result {
            self.status = e.clone();
        }
        result
    }

    /// Give `node` a support from the preset list.
    pub fn apply_support(&mut self, node: u32, preset: SupportPreset) -> Result<(), String> {
        let frame = self.problem.has_frame(node);
        self.commit(Edit::SetSupport { node, fixed: preset.fixed(frame) }).map(|_| ())
    }
    /// Give `node` the default load (redirected into the plane of a planar model).
    pub fn apply_default_load(&mut self, node: u32) -> Result<(), String> {
        let (force, moment) = (self.default_force(), [0.0; 3]);
        self.commit(Edit::SetLoad { node, force, moment }).map(|_| ())
    }
    pub fn remove_load(&mut self, node: u32) -> Result<(), String> {
        self.commit(Edit::SetLoad { node, force: [0.0; 3], moment: [0.0; 3] }).map(|_| ())
    }

    /// The default load, redirected to lie in the plane of a planar model (gravity-like: -z, or -y for an xy model).
    fn default_force(&self) -> [f64; 3] {
        let f = self.defaults.force;
        match self.problem.plane {
            Some(axis) if f[axis] != 0.0 => {
                let mut g = [0.0; 3];
                g[if axis == 2 { 1 } else { 2 }] = -f.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
                g
            }
            _ => f,
        }
    }

    /// A frame member's reference axis: the default direction unless it is parallel to the member.
    fn default_reference(&self, a: u32, b: u32) -> [f64; 3] {
        let (pa, pb) = (self.problem.node(a).map_or([0.0; 3], |n| n.pos), self.problem.node(b).map_or([0.0; 3], |n| n.pos));
        let d: [f64; 3] = std::array::from_fn(|k| pb[k] - pa[k]);
        let r = self.defaults.reference;
        let cross = [d[1] * r[2] - d[2] * r[1], d[2] * r[0] - d[0] * r[2], d[0] * r[1] - d[1] * r[0]];
        let parallel = cross.iter().all(|c| c.abs() <= 1e-9 * d.iter().fold(0.0_f64, |m, v| m.max(v.abs())).max(1e-300));
        if parallel {
            [0.0, 1.0, 0.0]
        } else {
            r
        }
    }

    pub fn delete_selected(&mut self) -> Result<(), String> {
        let all = self.selected_all();
        if all.is_empty() {
            return Err("nothing selected".into());
        }
        // Members first: removing a node also removes its members.
        let members = all.iter().filter_map(|h| if let Hit::Member(id) = h { Some(Edit::RemoveMember { id: *id }) } else { None });
        let nodes = all.iter().filter_map(|h| if let Hit::Node(id) = h { Some(Edit::RemoveNode { id: *id }) } else { None });
        let edits: Vec<Edit> = members.chain(nodes).collect();
        // A member already removed with its node must not fail the group: apply what still exists.
        let live: Vec<Edit> = edits
            .into_iter()
            .filter(|e| match e {
                Edit::RemoveMember { id } => self.problem.member(*id).is_some(),
                Edit::RemoveNode { id } => self.problem.node(*id).is_some(),
                _ => true,
            })
            .collect();
        self.commit_all(live)
    }

    /// Rows editable for the current selection.
    pub fn properties(&self) -> Vec<Property> {
        let row = |label: &str, value: f64, kind| Property { label: label.into(), value, kind };
        match self.selection {
            Some(Hit::Node(id)) => {
                let Some(n) = self.problem.node(id) else { return vec![] };
                let frame = self.problem.has_frame(id);
                let load = self.problem.load(id);
                let fixed = self.problem.support(id).map(|s| s.fixed).unwrap_or([false; 6]);
                let mut rows = vec![
                    row("x", n.pos[0], PropertyKind::Coordinate(0)),
                    row("y", n.pos[1], PropertyKind::Coordinate(1)),
                    row("z", n.pos[2], PropertyKind::Coordinate(2)),
                ];
                for k in 0..3 {
                    rows.push(row(["Fx", "Fy", "Fz"][k], load.map_or(0.0, |l| l.force[k]), PropertyKind::Force(k)));
                }
                if frame {
                    for k in 0..3 {
                        rows.push(row(["Mx", "My", "Mz"][k], load.map_or(0.0, |l| l.moment[k]), PropertyKind::Moment(k)));
                    }
                }
                for k in 0..if frame { 6 } else { 3 } {
                    rows.push(row(["fix ux", "fix uy", "fix uz", "fix rx", "fix ry", "fix rz"][k], f64::from(fixed[k]), PropertyKind::Fixed(k)));
                }
                rows
            }
            Some(Hit::Member(id)) => match self.problem.member(id).map(|m| m.kind) {
                Some(MemberKind::Truss { e, area }) => vec![row("E", e, PropertyKind::TrussE), row("area", area, PropertyKind::TrussArea)],
                Some(MemberKind::Frame { section: s, uniform_load: q, .. }) => vec![
                    row("E", s.e, PropertyKind::SecE),
                    row("G", s.g, PropertyKind::SecG),
                    row("area", s.area, PropertyKind::SecArea),
                    row("Iy", s.iy, PropertyKind::SecIy),
                    row("Iz", s.iz, PropertyKind::SecIz),
                    row("J (torsion)", s.torsion, PropertyKind::SecTorsion),
                    row("q local x", q[0], PropertyKind::Distributed(0)),
                    row("q local y", q[1], PropertyKind::Distributed(1)),
                    row("q local z", q[2], PropertyKind::Distributed(2)),
                ],
                None => vec![],
            },
            None => vec![row(&format!("plane {} =", self.plane_name()), self.plane_value, PropertyKind::PlaneOffset)],
        }
    }

    /// Set one property of the selection to `value` (validated and undoable).
    pub fn set_property(&mut self, kind: PropertyKind, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err("value must be a finite number".into());
        }
        let edit = match (self.selection, kind) {
            (_, PropertyKind::MoveGroup(axis)) => return self.move_group(axis, value),
            (_, PropertyKind::PlaneOffset) => {
                self.plane_value = value;
                self.status = format!("Working plane {} = {}", self.plane_name(), value);
                return Ok(());
            }
            (Some(Hit::Node(id)), PropertyKind::Coordinate(k)) => {
                let mut pos = self.problem.node(id).ok_or("no such node")?.pos;
                pos[k] = value;
                Edit::MoveNode { id, pos }
            }
            (Some(Hit::Node(id)), PropertyKind::Force(k) | PropertyKind::Moment(k)) => {
                let (mut force, mut moment) = self.problem.load(id).map_or(([0.0; 3], [0.0; 3]), |l| (l.force, l.moment));
                if matches!(kind, PropertyKind::Force(_)) {
                    force[k] = value;
                } else {
                    moment[k] = value;
                }
                Edit::SetLoad { node: id, force, moment }
            }
            (Some(Hit::Node(id)), PropertyKind::Fixed(k)) => {
                let mut fixed = self.problem.support(id).map(|s| s.fixed).unwrap_or([false; 6]);
                fixed[k] = value != 0.0;
                Edit::SetSupport { node: id, fixed }
            }
            (Some(Hit::Member(id)), _) => {
                let spec = self.problem.member(id).ok_or("no such member")?;
                let mut k2 = spec.kind;
                match (&mut k2, kind) {
                    (MemberKind::Truss { e, .. }, PropertyKind::TrussE) => *e = value,
                    (MemberKind::Truss { area, .. }, PropertyKind::TrussArea) => *area = value,
                    (MemberKind::Frame { section, .. }, PropertyKind::SecE) => section.e = value,
                    (MemberKind::Frame { section, .. }, PropertyKind::SecG) => section.g = value,
                    (MemberKind::Frame { section, .. }, PropertyKind::SecArea) => section.area = value,
                    (MemberKind::Frame { section, .. }, PropertyKind::SecIy) => section.iy = value,
                    (MemberKind::Frame { section, .. }, PropertyKind::SecIz) => section.iz = value,
                    (MemberKind::Frame { section, .. }, PropertyKind::SecTorsion) => section.torsion = value,
                    (MemberKind::Frame { uniform_load, .. }, PropertyKind::Distributed(k)) => uniform_load[k] = value,
                    _ => return Err("that property does not apply to this member".into()),
                }
                Edit::SetKind { id, kind: k2 }
            }
            _ => return Err("that property does not apply to the selection".into()),
        };
        self.commit(edit).map(|_| ())
    }

    /// Convert the selected member between truss and frame, keeping its connectivity.
    pub fn convert_selected(&mut self, to_frame: bool) -> Result<(), String> {
        let Some(Hit::Member(id)) = self.selection else { return Err("select a member first".into()) };
        let spec = self.problem.member(id).ok_or("no such member")?.clone();
        if spec.kind.is_frame() == to_frame {
            return Err(format!("member {id} is already a {}", if to_frame { "frame" } else { "truss" }));
        }
        let kind = if to_frame {
            MemberKind::Frame { section: self.defaults.section, reference: self.default_reference(spec.nodes[0], spec.nodes[1]), uniform_load: [0.0; 3] }
        } else {
            MemberKind::Truss { e: self.defaults.truss.0, area: self.defaults.truss.1 }
        };
        self.commit(Edit::SetKind { id, kind }).map(|_| ())
    }

    /// Replace the whole problem (load/new); undoable.
    pub fn replace(&mut self, problem: MemberProblem, mark_saved: bool) {
        self.undo.push(std::mem::replace(&mut self.problem, problem));
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.selection = None;
        self.pending = None;
        self.frame = Frame::fit(&self.problem);
        if mark_saved {
            self.saved = Some(self.problem.clone());
        }
        self.mark();
    }

    /// Save as JSON (written beside the target, then renamed, so a failed write keeps the old file).
    pub fn save(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.problem.validate()?;
        let text = serde_json::to_string_pretty(&self.problem).map_err(|e| e.to_string())?;
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| format!("cannot write next to {}: {e}", path.display()))?;
        std::io::Write::write_all(&mut tmp, text.as_bytes()).map_err(|e| e.to_string())?;
        tmp.persist(path).map_err(|e| e.error.to_string())?;
        self.saved = Some(self.problem.clone());
        self.dirty = false;
        Ok(())
    }
    pub fn load(&mut self, path: &std::path::Path) -> Result<(), String> {
        let problem = read_problem(path)?;
        self.replace(problem, true);
        Ok(())
    }
}

/// Read and validate a member problem file (size-capped).
pub fn read_problem(path: &std::path::Path) -> Result<MemberProblem, String> {
    const LIMIT: u64 = 32 * 1024 * 1024;
    let len = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if len > LIMIT {
        return Err("member project file exceeds 32 MiB".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let problem: MemberProblem = serde_json::from_str(&text).map_err(|e| format!("not a member project: {e}"))?;
    problem.validate()?;
    Ok(problem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fea_problem::members::templates;

    fn blank() -> EditorState {
        EditorState::new(MemberProblem::new("t"))
    }
    fn plane(x: f64, y: f64) -> Option<[f64; 3]> {
        Some([x, y, 0.0])
    }

    #[test]
    fn working_plane_offset_is_editable_with_nothing_selected() {
        let mut s = blank();
        s.set_tool(Tool::Node);
        let rows = s.properties();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, PropertyKind::PlaneOffset);
        s.set_property(PropertyKind::PlaneOffset, 30.0).unwrap();
        s.click(None, plane(10.0, 10.0)).unwrap();
        assert_eq!(s.problem.nodes[0].pos, [10.0, 10.0, 30.0]);
        assert!(s.set_property(PropertyKind::PlaneOffset, f64::NAN).is_err());
    }

    #[test]
    fn node_tool_places_on_the_snapped_working_plane() {
        let mut s = blank();
        s.set_tool(Tool::Node);
        s.click(None, plane(12.4, -7.6)).unwrap();
        assert_eq!(s.problem.nodes[0].pos, [10.0, -10.0, 0.0]);
        assert_eq!(s.selection, Some(Hit::Node(1)));
        // Next working plane is x = const; the pinned coordinate is not snapped, the others are.
        s.cycle_plane();
        assert_eq!(s.plane_name(), "x");
        s.plane_value = 25.0;
        s.set_tool(Tool::Node);
        s.click(None, Some([4.0, 3.0, 9.0])).unwrap();
        assert_eq!(s.problem.nodes[1].pos, [25.0, 5.0, 10.0]);
    }
    #[test]
    fn edge_on_view_explains_instead_of_guessing() {
        let mut s = blank();
        s.set_tool(Tool::Node);
        let e = s.click(None, None).unwrap_err();
        assert!(e.contains("edge-on") && s.problem.nodes.is_empty() && s.status == e);
    }
    #[test]
    fn member_tools_connect_nodes_with_a_pending_first_click() {
        let mut s = blank();
        s.set_tool(Tool::Truss);
        s.click(None, plane(0.0, 0.0)).unwrap();
        assert_eq!(s.pending, Some(1));
        s.click(None, plane(50.0, 0.0)).unwrap();
        assert_eq!((s.problem.nodes.len(), s.problem.members.len(), s.pending), (2, 1, None));
        assert_eq!(s.selection, Some(Hit::Member(3)));
        // Clicking the same node twice cancels; a duplicate pair is refused and keeps the pending start.
        s.click(Some(Hit::Node(1)), None).unwrap();
        s.click(Some(Hit::Node(1)), None).unwrap();
        assert_eq!(s.pending, None);
        s.click(Some(Hit::Node(1)), None).unwrap();
        assert!(s.click(Some(Hit::Node(2)), None).is_err());
        assert_eq!(s.pending, Some(1));
        assert_eq!(s.problem.members.len(), 1);
        assert!(s.click(Some(Hit::Member(3)), None).is_err());
    }
    #[test]
    fn frame_tool_avoids_a_parallel_reference_axis() {
        let mut s = blank();
        s.set_tool(Tool::Frame);
        s.click(None, plane(0.0, 0.0)).unwrap();
        s.plane_axis = 0; // member along z needs a non-z reference
        s.plane_value = 0.0;
        s.click(None, Some([0.0, 0.0, 50.0])).unwrap();
        let MemberKind::Frame { reference, .. } = s.problem.members[0].kind else { panic!() };
        assert_eq!(reference, [0.0, 1.0, 0.0]);
        s.problem.validate().unwrap();
    }
    #[test]
    fn support_and_load_tools_offer_a_list_for_the_clicked_node() {
        let mut s = EditorState::new(templates().remove(1));
        let tip = s.problem.nodes[1].id;
        s.set_tool(Tool::Support);
        s.click(Some(Hit::Node(tip)), None).unwrap();
        assert_eq!(s.popup.take(), Some((Tool::Support, tip)));
        assert_eq!(s.selection, Some(Hit::Node(tip)));
        assert!(s.problem.support(tip).is_none(), "the click alone changes nothing");
        s.apply_support(tip, SupportPreset::Fixed).unwrap();
        assert_eq!(s.problem.support(tip).unwrap().fixed, [true; 6], "frame node -> fixed");
        s.apply_support(tip, SupportPreset::Roller(0)).unwrap();
        assert_eq!(s.problem.support(tip).unwrap().fixed, [false, true, true, false, false, false]);
        s.apply_support(tip, SupportPreset::Pinned).unwrap();
        assert_eq!(s.problem.support(tip).unwrap().fixed, [true, true, true, false, false, false]);
        s.apply_support(tip, SupportPreset::Remove).unwrap();
        assert!(s.problem.support(tip).is_none());
        assert!(s.click(None, plane(0.0, 0.0)).is_err(), "empty space is not a node");
        s.set_tool(Tool::Load);
        let root = s.problem.nodes[0].id;
        s.click(Some(Hit::Node(root)), None).unwrap();
        assert_eq!(s.popup.take(), Some((Tool::Load, root)));
        s.apply_default_load(root).unwrap();
        assert_eq!(s.problem.load(root).unwrap().force, [0.0, 0.0, -100.0]);
        s.remove_load(root).unwrap();
        assert!(s.problem.load(root).is_none());
        // In a planar xy model the default load is redirected into the plane instead of being refused.
        let mut p = EditorState::new(fea_problem::members::blank_planar());
        p.set_tool(Tool::Node);
        p.click(None, plane(0.0, 0.0)).unwrap();
        p.apply_default_load(1).unwrap();
        assert_eq!(p.problem.load(1).unwrap().force, [0.0, -100.0, 0.0]);
    }
    #[test]
    fn undo_redo_walk_history_and_new_edits_clear_redo() {
        let mut s = blank();
        s.set_tool(Tool::Node);
        s.click(None, plane(0.0, 0.0)).unwrap();
        s.click(None, plane(10.0, 0.0)).unwrap();
        assert_eq!(s.problem.nodes.len(), 2);
        assert!(s.undo() && s.problem.nodes.len() == 1);
        assert!(s.redo() && s.problem.nodes.len() == 2);
        assert!(!s.redo());
        s.undo();
        s.click(None, plane(20.0, 0.0)).unwrap();
        assert!(!s.can_redo(), "a new edit discards the redo branch");
        while s.undo() {}
        assert!(s.problem.nodes.is_empty() && !s.can_undo());
        assert!(!s.dirty, "undoing everything returns to the saved state");
    }
    #[test]
    fn failed_and_noop_edits_leave_problem_and_history_alone() {
        let mut p = MemberProblem::new("two");
        p.apply(Edit::AddNode { pos: [0.0; 3] }).unwrap();
        p.apply(Edit::AddNode { pos: [10.0, 0.0, 0.0] }).unwrap();
        let mut s = EditorState::new(p.clone());
        s.select(Some(Hit::Node(2)));
        // nonfinite, coincident-with-another-node and wrong-kind edits are refused
        assert!(s.set_property(PropertyKind::Coordinate(0), f64::NAN).is_err());
        assert!(s.set_property(PropertyKind::Coordinate(0), 0.0).is_err());
        assert!(s.set_property(PropertyKind::TrussE, 1.0).is_err());
        assert_eq!(s.problem, p);
        assert!(!s.can_undo() && !s.dirty);
        assert!(!s.status.is_empty(), "the refusal is explained");
        // setting a property to its current value succeeds but records nothing
        s.set_property(PropertyKind::Coordinate(0), 10.0).unwrap();
        assert!(!s.can_undo() && !s.dirty);
        // a real edit is recorded, and undo restores the exact original
        s.set_property(PropertyKind::Coordinate(0), 12.0).unwrap();
        assert!(s.can_undo() && s.dirty);
        s.undo();
        assert_eq!(s.problem, p);
    }
    #[test]
    fn property_rows_follow_the_selection_and_edit_through_the_problem() {
        let mut s = EditorState::new(templates().remove(1));
        s.select(Some(Hit::Node(s.problem.nodes[1].id)));
        let rows = s.properties();
        assert_eq!(rows.iter().filter(|r| matches!(r.kind, PropertyKind::Moment(_))).count(), 3, "frame node shows moments");
        assert_eq!(rows.iter().find(|r| r.kind == PropertyKind::Force(1)).unwrap().value, 100.0);
        s.set_property(PropertyKind::Force(1), 250.0).unwrap();
        assert_eq!(s.problem.load(2).unwrap().force[1], 250.0);
        s.set_property(PropertyKind::Coordinate(0), 80.0).unwrap();
        assert_eq!(s.problem.nodes[1].pos[0], 80.0);
        s.set_property(PropertyKind::Fixed(2), 1.0).unwrap();
        assert!(s.problem.support(2).unwrap().fixed[2]);
        s.select(Some(Hit::Member(s.problem.members[0].id)));
        assert_eq!(s.properties().len(), 9);
        s.set_property(PropertyKind::SecArea, 7.0).unwrap();
        let MemberKind::Frame { section, .. } = s.problem.members[0].kind else { panic!() };
        assert_eq!(section.area, 7.0);
        assert!(s.set_property(PropertyKind::TrussE, 1.0).is_err());
        assert!(s.set_property(PropertyKind::SecArea, -1.0).is_err());
        assert!(s.problem.members[0].kind.is_frame());
        let t = EditorState::new(templates().remove(0));
        let mut t = t;
        t.select(Some(Hit::Node(t.problem.nodes[3].id)));
        assert!(t.properties().iter().all(|r| !matches!(r.kind, PropertyKind::Moment(_))), "truss node: no moments");
        assert_eq!(t.properties().iter().filter(|r| matches!(r.kind, PropertyKind::Fixed(_))).count(), 3);
    }
    #[test]
    fn delete_and_convert_selected() {
        let mut s = EditorState::new(templates().remove(0));
        assert!(s.delete_selected().is_err());
        s.select(Some(Hit::Node(s.problem.nodes[3].id)));
        s.delete_selected().unwrap();
        assert!(s.problem.members.is_empty() && s.selection.is_none());
        let mut f = EditorState::new(templates().remove(1));
        let id = f.problem.members[0].id;
        f.select(Some(Hit::Member(id)));
        assert!(f.convert_selected(true).is_err());
        f.convert_selected(false).unwrap();
        assert!(!f.problem.members[0].kind.is_frame());
        assert!(f.problem.support(f.problem.nodes[0].id).unwrap().fixed[3..].iter().all(|v| !v));
        f.convert_selected(true).unwrap();
        assert!(f.problem.members[0].kind.is_frame());
    }
    #[test]
    fn solutions_go_stale_when_the_problem_changes() {
        let mut s = EditorState::new(templates().remove(0));
        let solved = s.problem.solve().unwrap();
        s.accept_solution(solved, s.problem.clone());
        assert!(s.current_solution().is_some() && !s.solution_is_stale());
        s.select(Some(Hit::Node(s.problem.nodes[3].id)));
        s.set_property(PropertyKind::Force(2), -2000.0).unwrap();
        assert!(s.current_solution().is_none() && s.solution_is_stale());
        s.undo();
        assert!(s.current_solution().is_some(), "undo returns to the solved problem");
    }
    #[test]
    fn save_load_round_trip_validates_and_tracks_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.members.json");
        let mut s = EditorState::new(templates().remove(1));
        s.select(Some(Hit::Node(2)));
        s.set_property(PropertyKind::Force(1), 123.0).unwrap();
        assert!(s.dirty);
        s.save(&path).unwrap();
        assert!(!s.dirty);
        let mut t = blank();
        t.load(&path).unwrap();
        assert_eq!(t.problem, s.problem);
        assert!(!t.dirty && t.can_undo(), "load is itself undoable");
        t.undo();
        assert!(t.problem.nodes.is_empty());
        // Corrupt, invalid and oversized inputs are refused with messages.
        std::fs::write(&path, "{ nope").unwrap();
        assert!(t.load(&path).unwrap_err().contains("not a member project"));
        let mut bad = s.problem.clone();
        bad.members[0].nodes[1] = 777;
        std::fs::write(&path, serde_json::to_string(&bad).unwrap()).unwrap();
        assert!(t.load(&path).is_err());
        assert!(t.load(&dir.path().join("missing.json")).is_err());
        // A failed save keeps the previous file intact.
        let good = serde_json::to_string(&s.problem).unwrap();
        std::fs::write(&path, &good).unwrap();
        assert!(s.save(&dir.path().join("no/such/dir/p.json")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), good);
    }

    #[test]
    fn groups_select_toggle_move_and_delete_as_one_undo_step() {
        let mut s = EditorState::new(templates().remove(0));
        let ids: Vec<u32> = s.problem.nodes.iter().map(|n| n.id).collect();
        assert!(ids.len() >= 3);
        s.select(Some(Hit::Node(ids[0])));
        s.toggle(Hit::Node(ids[1]));
        assert_eq!(s.selected_all().len(), 2);
        s.toggle(Hit::Node(ids[1]));
        assert_eq!(s.selected_all(), [Hit::Node(ids[0])]);
        // Box result replaces, additive extends.
        s.select_many(vec![Hit::Node(ids[0]), Hit::Node(ids[1])], false);
        s.select_many(vec![Hit::Node(ids[2])], true);
        assert_eq!(s.selected_all().len(), 3);
        let before: Vec<[f64; 3]> = ids.iter().map(|i| s.problem.node(*i).unwrap().pos).collect();
        let undo = s.undo.len();
        s.move_group(0, 5.0).unwrap();
        assert_eq!(s.undo.len(), undo + 1, "one undo step");
        for (k, id) in ids.iter().enumerate().take(3) {
            assert_eq!(s.problem.node(*id).unwrap().pos[0], before[k][0] + 5.0);
        }
        assert!(s.undo());
        assert_eq!(s.problem.node(ids[0]).unwrap().pos, before[0]);
        assert!(s.move_group(0, f64::NAN).is_err());
        // Deleting a group removes nodes and (with them) their members in one step; the selection empties.
        let (nodes, undo) = (s.problem.nodes.len(), s.undo.len());
        s.delete_selected().unwrap();
        assert_eq!(s.problem.nodes.len(), nodes - 3);
        assert_eq!(s.undo.len(), undo + 1);
        assert!(s.selected_all().is_empty());
        assert!(s.undo());
        assert_eq!(s.problem.nodes.len(), nodes);
        s.problem.validate().unwrap();
    }

    #[test]
    fn a_box_selects_nodes_inside_and_members_with_both_ends_inside() {
        let s = EditorState::new(templates().remove(0));
        let cam = crate::gpu_viewer::editor::camera::Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
        let size = [800.0, 800.0];
        let all = super::super::camera::pick_box(&s.problem, &s.frame, &cam, [0.0, 0.0], size, size);
        assert_eq!(all.iter().filter(|h| matches!(h, Hit::Node(_))).count(), s.problem.nodes.len());
        assert_eq!(all.iter().filter(|h| matches!(h, Hit::Member(_))).count(), s.problem.members.len());
        assert!(super::super::camera::pick_box(&s.problem, &s.frame, &cam, [0.0, 0.0], [1.0, 1.0], size).is_empty());
    }
}
