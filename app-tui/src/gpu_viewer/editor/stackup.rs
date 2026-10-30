//! Native stack sizing. Calculations live in bushing-solver, never in the painter.
use super::{
    authoring::Authoring,
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods, Tone},
};
use crate::gpu_viewer::{
    bench::Bench,
    controller::UiAction,
    menu::{Entry, Item, Menu},
    ui::*,
    Scene,
};
use bushing_solver::stackup::{self, AngularityMode, Inputs, Member, ResultSet, Status, WallMode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// OS-owned lock is released even after a crash; a case cannot open twice.
pub struct WindowLease(std::fs::File);
impl WindowLease {
    pub fn acquire(path: &Path) -> Result<Self, String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("window.lock"))
            .map_err(|e| format!("Cannot reserve stackup window: {e}"))?;
        file.try_lock()
            .map_err(|e| format!("Stackup case is already open or cannot be locked: {e}"))?;
        Ok(Self(file))
    }
}
impl Drop for WindowLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackupAct {
    Select(usize),
    Add,
    Remove,
    Up,
    Down,
    Common,
    ToggleBushing,
    Mode,
    WallMode,
    AdoptImported,
    ClassifyImported(WallMode),
    Recalculate,
    Save,
    Send,
}
fn action(a: StackupAct) -> UiAction {
    UiAction::Editor(EditorAction::Stackup(a))
}
#[derive(Serialize, Deserialize)]
struct StackupFile {
    version: u32,
    inputs: Inputs,
}
#[derive(Serialize, Deserialize)]
struct Recovery {
    file: StackupFile,
    drafts: Vec<(u64, u8, String)>,
    selected: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrecisionPage {
    pub inputs: Inputs,
    pub result: Option<Arc<ResultSet>>,
    pub selected: usize,
    pub current: bool,
    pub error: Option<String>,
}

pub struct StackupSession {
    pub inputs: Inputs,
    selected: usize,
    drafts: BTreeMap<(u64, u8), String>,
    path: PathBuf,
    file_revision: Option<String>,
    dirty: bool,
    input_revision: u64,
    result_revision: Option<u64>,
    result: Result<Arc<ResultSet>, String>,
    status: String,
}
impl StackupSession {
    pub fn open(path: &Path) -> Result<Self, String> {
        let inputs = if path.exists() {
            let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let f:StackupFile=serde_json::from_str(&raw).map_err(|e|format!("Stackup file requires version 3 common-axis cleanup and optional bushing policy; legacy preliminary cases must be reviewed: {e}"))?;
            if f.version != 3 {
                return Err("Unsupported stackup version; earlier preliminary geometry cannot be silently accepted".into());
            }
            f.inputs
        } else {
            Inputs::default()
        };
        if inputs.members.is_empty() {
            return Err("Stackup must contain at least one member".into());
        }
        let mut s = Self {
            inputs,
            selected: 0,
            drafts: BTreeMap::new(),
            path: path.into(),
            file_revision: crate::persistence::revision(path),
            dirty: false,
            input_revision: 0,
            result_revision: None,
            result: Err("Recalculate to evaluate geometry".into()),
            status: "Inputs recalculate on Enter or leaving a field. Inches and degrees.".into(),
        };
        s.recalculate();
        Ok(s)
    }
    fn current(&self) -> bool {
        self.result_revision == Some(self.input_revision)
    }
    fn changed(&mut self) {
        self.input_revision += 1;
        self.dirty = true;
        self.status = "Editing inputs. Press Enter or leave the field to update results.".into();
    }
    fn recalculate(&mut self) {
        if self.drafts.iter().any(|((_, id), text)| {
            (self.inputs.bushing_enabled || !bushing_field(*id)) && !valid_text(*id, text)
        }) {
            self.result =
                Err("Invalid input text. Correct highlighted fields before recalculating.".into());
            self.result_revision = None;
            return;
        }
        self.result = stackup::calculate(&self.inputs).map(Arc::new).map_err(|e| {
            format!(
                "{:?}, member {:?}, {}: {}",
                e.kind, e.member, e.parameter, e.message
            )
        });
        self.result_revision = Some(self.input_revision);
        self.status = if self.result.is_ok() {
            "Calculation current. Check each member's status and governing constraint."
        } else {
            "Calculation failed; no accepted design."
        }
        .into();
    }
    fn key_for(&self, id: u8) -> (u64, u8) {
        (
            if id >= 10 {
                self.inputs.members[self.selected].id
            } else {
                0
            },
            id,
        )
    }
    fn value(&self, id: u8) -> String {
        if let Some(t) = self.drafts.get(&self.key_for(id)) {
            return t.clone();
        }
        let i = &self.inputs;
        let m = &i.members[self.selected];
        match id {
            0 => i.finished_id.nominal.to_string(),
            1 => i.finished_id.min.to_string(),
            2 => i.finished_id.max.to_string(),
            3 => i.common_wall.map(|v| v.to_string()).unwrap_or_default(),
            4 => i.pilot_z.to_string(),
            5 => i.pilot_offset.to_string(),
            10 => m.name.clone(),
            11 => m.thickness.to_string(),
            12 => m.hole.nominal.to_string(),
            13 => m.hole.min.to_string(),
            14 => m.hole.max.to_string(),
            15 => m.seating_min.to_string(),
            16 => m.seating_max.to_string(),
            17 => m.od_tolerance.to_string(),
            18 => m.reference_normal_deg.to_string(),
            19 => m.original.degrees.to_string(),
            20 => m.finished.degrees.to_string(),
            21 => m.original.zone_diameter.to_string(),
            22 => m.finished.zone_diameter.to_string(),
            23 => m.original.reference_span.to_string(),
            24 => m.finished.reference_span.to_string(),
            25 => m.wall_override.map(|v| v.to_string()).unwrap_or_default(),
            26 => m.housing_material.clone(),
            27 => m.bushing_material.clone(),
            28 => m.finished_id_min.map(|v| v.to_string()).unwrap_or_default(),
            30 => m.member_width.to_string(),
            31 => m.member_depth.to_string(),
            29 => m.finished_id_max.map(|v| v.to_string()).unwrap_or_default(),
            _ => String::new(),
        }
    }
    fn field(&self, id: u8, label: &str) -> DocInput {
        let text = self.value(id);
        let valid = valid_text(id, &text);
        DocInput {
            id,
            label: label.into(),
            text,
            hint: if id == 3 || id == 25 {
                "blank = unset"
            } else {
                ""
            }
            .into(),
            error: (!valid).then(|| "Finite numeric value required".into()),
            note: match id {
                4 => Some("Axial distance from stack entrance. For 1 in thickness: 0 = entry, 0.5 = midpoint, 1 = exit.".into()),
                5 => Some("Maximum relative separation of the original and finished axis centers at the pilot; radial distance.".into()),
                18 => Some("Angular datum measured from +Z toward +X. 0.5 means 0.5 degrees, not a position through the thickness.".into()),
                _ => None,
            },
            submit: Some(action(StackupAct::Recalculate)),
            commit_on_blur: true,
            grow: 1.0,
            trailing: None,
            help: None,
            choices: vec![],
        }
    }
    fn save(&mut self) {
        if self.drafts.iter().any(|((_, id), t)| {
            (self.inputs.bushing_enabled || !bushing_field(*id)) && !valid_text(*id, t)
        }) {
            self.status = "Cannot save invalid input text".into();
            return;
        }
        if crate::persistence::revision(&self.path) != self.file_revision {
            self.status = "Save stopped. File changed outside this window.".into();
            return;
        }
        let f = StackupFile {
            version: 3,
            inputs: self.inputs.clone(),
        };
        match serde_json::to_string_pretty(&f)
            .map_err(|e| e.to_string())
            .and_then(|s| crate::persistence::write(&self.path, s).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.file_revision = crate::persistence::revision(&self.path);
                self.dirty = false;
                self.status = format!("Saved {}", self.path.display());
            }
            Err(e) => self.status = e,
        }
    }
    fn transfer_availability(&self) -> Result<(), String> {
        if !self.inputs.bushing_enabled {
            return Err("Enable optional bushing sizing first".into());
        }
        if !self.current() {
            return Err("Results stale. Recalculate first.".into());
        }
        let r = self.result.as_ref().map_err(Clone::clone)?;
        let layer = &r.layers[self.selected];
        if !matches!(layer.status, Status::Feasible | Status::Informational) {
            return Err(
                "Only current feasible or explicitly informational geometry can transfer".into(),
            );
        }
        if r.finished_angles != [0.0, 0.0] {
            return Err("Structural workbench supports parallel bores only. Oblique bore or angular uncertainty cannot transfer losslessly.".into());
        }
        if self.inputs.finished_id.min != self.inputs.finished_id.max {
            return Err("Structural target has no ID tolerance band; source limits must not be silently discarded.".into());
        }
        Ok(())
    }
    fn send(&mut self) {
        if let Err(e) = self.transfer_availability() {
            self.status = e;
            return;
        }
        let file = match super::stackup_transfer::prepare(
            &self.inputs,
            self.selected,
            &self.path.to_string_lossy(),
            self.input_revision,
        ) {
            Ok(file) => file,
            Err(e) => {
                self.status = format!("Transfer refused: {e}");
                return;
            }
        };
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut directory = None;
        for n in 1..=1000 {
            let dir = parent.join(format!(
                "stackup-transfer-{}-{}-{n}",
                self.inputs.members[self.selected].id, self.input_revision
            ));
            match std::fs::create_dir(&dir) {
                Ok(()) => {
                    directory = Some(dir);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => {
                    self.status = format!("Could not create transfer folder: {e}");
                    return;
                }
            }
        }
        let Some(dir) = directory else {
            self.status = "No free transfer folder name".into();
            return;
        };
        let result = crate::persistence::write(dir.join("bushing.json"), file.to_json());
        if let Err(e) = result {
            self.status = format!("Could not write transfer: {e}");
            return;
        }
        match super::handoff::launch(crate::gpu_viewer::EditorKind::Bushing,&dir.join("bushing.json")) {
            Ok(())=>self.status=format!("Opened member #{} in native Bushing Workbench. Calculated member hole, enlargement reason and wall context preserved.",self.inputs.members[self.selected].id),
            Err(e)=>self.status=format!("Transfer saved at {}, launch failed: {e}",dir.display()),
        }
    }
    fn act_stackup(&mut self, a: StackupAct) {
        match a {
            StackupAct::Select(i) if i < self.inputs.members.len() => {
                self.selected = i;
                return;
            }
            StackupAct::Add => {
                let Some(id) = self
                    .inputs
                    .members
                    .iter()
                    .map(|m| m.id)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                else {
                    self.status = "Member identifiers exhausted; member not added".into();
                    return;
                };
                self.inputs.members.push(Member::new(id));
                self.selected = self.inputs.members.len() - 1;
            }
            StackupAct::Remove if self.inputs.members.len() > 1 => {
                let id = self.inputs.members.remove(self.selected).id;
                self.drafts.retain(|(member, _), _| *member != id);
                self.selected = self.selected.min(self.inputs.members.len() - 1);
            }
            StackupAct::Up if self.selected > 0 => {
                self.inputs.members.swap(self.selected, self.selected - 1);
                self.selected -= 1;
            }
            StackupAct::Down if self.selected + 1 < self.inputs.members.len() => {
                self.inputs.members.swap(self.selected, self.selected + 1);
                self.selected += 1;
            }
            StackupAct::ToggleBushing => self.inputs.bushing_enabled = !self.inputs.bushing_enabled,
            StackupAct::Common => self.inputs.uniform_od = !self.inputs.uniform_od,
            StackupAct::Mode => {
                self.inputs.mode = match self.inputs.mode {
                    AngularityMode::Degrees => AngularityMode::CylindricalZone,
                    AngularityMode::CylindricalZone => AngularityMode::Degrees,
                }
            }
            StackupAct::WallMode => {
                let m = &mut self.inputs.members[self.selected];
                m.wall_mode = match m.wall_mode {
                    WallMode::Required => WallMode::Informational,
                    WallMode::Informational => WallMode::Unclassified,
                    WallMode::Unclassified => WallMode::Required,
                };
            }
            StackupAct::AdoptImported => {
                let m = &mut self.inputs.members[self.selected];
                if let Some(w) = &m.imported_wall {
                    m.wall_override = Some(w.value);
                    m.wall_mode = w.semantics;
                    m.imported_use = (w.semantics != WallMode::Unclassified).then_some(w.semantics);
                    self.drafts.remove(&(m.id, 25));
                }
            }
            StackupAct::ClassifyImported(mode) => {
                let m = &mut self.inputs.members[self.selected];
                if let Some(w) = &m.imported_wall {
                    m.wall_override = Some(w.value);
                    m.wall_mode = mode;
                    m.imported_use = Some(mode);
                    self.drafts.remove(&(m.id, 25));
                }
            }
            StackupAct::Recalculate => {
                self.recalculate();
                return;
            }
            StackupAct::Save => {
                self.save();
                return;
            }
            StackupAct::Send => {
                self.send();
                return;
            }
            _ => return,
        }
        self.changed();
        self.recalculate();
    }
    fn drawing(&self, r: &ResultSet) -> DocDrawing {
        let mut paths = Vec::new();
        for (index, ((geometry, layer), m)) in r
            .section
            .layers
            .iter()
            .zip(&r.layers)
            .zip(&self.inputs.members)
            .enumerate()
        {
            paths.push(DocPath {
                points: geometry.od.to_vec(),
                label: format!(
                    "{} #{} {:.4} in {:?}",
                    m.name, m.id, m.thickness, layer.status
                ),
                kind: 0,
                selected: index == self.selected,
                select: Some(action(StackupAct::Select(index))),
            });
            for (walls, kind, label) in [
                (&geometry.original, 1, "Original hole"),
                (&geometry.finished, 3, "Finished bore"),
            ] {
                for wall in walls {
                    paths.push(DocPath {
                        points: wall.to_vec(),
                        label: label.into(),
                        kind,
                        selected: false,
                        select: None,
                    });
                }
            }
        }
        for (points, kind, label) in [
            (&r.section.original_axis, 1, "Original common axis"),
            (&r.section.finished_axis, 2, "Finished common axis"),
        ] {
            paths.push(DocPath {
                points: points.to_vec(),
                label: label.into(),
                kind,
                selected: false,
                select: None,
            });
        }
        DocDrawing {
            bounds: r.section.bounds,
            paths,
        }
    }
}
fn bushing_field(id: u8) -> bool {
    matches!(id,0..=3|25..=29)
}
fn valid_text(id: u8, text: &str) -> bool {
    matches!(id, 10 | 26 | 27)
        || (matches!(id, 3 | 25 | 28 | 29) && text.trim().is_empty())
        || text.parse::<f64>().is_ok_and(f64::is_finite)
}
fn columns(names: &[&str], width: f32) -> Vec<DocColumn> {
    names
        .iter()
        .map(|n| DocColumn {
            title: (*n).into(),
            width,
            right: false,
            sort: None,
            sorted: None,
        })
        .collect()
}
pub(crate) fn optional(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.5} in"))
        .unwrap_or_else(|| "Unset".into())
}
impl Authoring for StackupSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        vec![]
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn act(&mut self, a: EditorAction) {
        if let EditorAction::Stackup(a) = a {
            self.act_stackup(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        if matches!(key,Key::Char(c) if mods.command&&c.eq_ignore_ascii_case(&'s')) {
            self.save();
            KeyResult::Handled
        } else {
            KeyResult::Unhandled
        }
    }
    fn click(&mut self, _: [f32; 2], _: [f32; 2], _: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _: [f32; 2], _: [f32; 2], _: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        Ok(Scene {
            label: "bushing-stackup".into(),
            positions: vec![[0.0; 3]; 3],
            triangles: vec![0, 1, 2],
            samples: vec![[0.0; 4]; 3],
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: String::new(),
            fields: vec![],
        })
    }
    fn recovery_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&Recovery {
            file: StackupFile {
                version: 3,
                inputs: self.inputs.clone(),
            },
            drafts: self
                .drafts
                .iter()
                .map(|((member, id), text)| (*member, *id, text.clone()))
                .collect(),
            selected: self.selected,
        })
        .map_err(|e| e.to_string())
    }
    fn restore_recovery(&mut self) -> Result<(), String> {
        let raw = std::fs::read_to_string(self.path.with_extension("recovery.json"))
            .map_err(|e| e.to_string())?;
        let recovery: Recovery = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if recovery.file.version != 3 || recovery.file.inputs.members.is_empty() {
            return Err("Unsupported or empty recovery file".into());
        }
        self.inputs = recovery.file.inputs;
        self.selected = recovery.selected.min(self.inputs.members.len() - 1);
        self.drafts = recovery
            .drafts
            .into_iter()
            .map(|(member, id, text)| ((member, id), text))
            .collect();
        self.changed();
        self.recalculate();
        Ok(())
    }
    fn needs_scene(&self) -> bool {
        false
    }
    fn poll(&mut self) -> bool {
        false
    }
    fn prompting(&self) -> bool {
        false
    }
    fn solving(&self) -> bool {
        false
    }
    fn status(&self) -> String {
        self.status.clone()
    }
    fn set_status(&mut self, t: String) {
        self.status = t;
    }
    fn title(&self) -> String {
        format!("Bushing Stackup{}", if self.dirty { " *" } else { "" })
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn geometry_view(&self) -> bool {
        false
    }
    fn tree(&self) -> Vec<TreeNode> {
        self.inputs
            .members
            .iter()
            .enumerate()
            .map(|(index, m)| {
                TreeNode::new(
                    index as u32 + 1,
                    0,
                    'o',
                    format!("{} / #{} {}", index + 1, m.id, m.name),
                )
                .selected(index == self.selected)
                .on_click(action(StackupAct::Select(index)))
            })
            .collect()
    }
    fn properties(&self) -> Vec<DockItem> {
        vec![]
    }
    fn ribbon(&self) -> Vec<RibbonTab> {
        vec![]
    }
    fn menus(&self) -> Vec<Menu> {
        vec![]
    }
    fn context_menu(&self) -> Vec<Item> {
        vec![]
    }
    fn has_document(&self) -> bool {
        true
    }
    fn document_field(&mut self, id: u8, text: String) {
        let key = self.key_for(id);
        if self.value(id) == text {
            return;
        }
        self.drafts.insert(key, text.clone());
        self.changed();
        if !valid_text(id, &text) {
            return;
        }
        let value = text.parse::<f64>().unwrap_or(0.0);
        let m = &mut self.inputs.members[self.selected];
        match id {
            0 => {
                let band = &mut self.inputs.finished_id;
                if band.min == band.nominal && band.max == band.nominal {
                    *band = stackup::Band::exact(value);
                    self.drafts.remove(&(0, 1));
                    self.drafts.remove(&(0, 2));
                } else {
                    band.nominal = value;
                }
            }
            1 => self.inputs.finished_id.min = value,
            2 => self.inputs.finished_id.max = value,
            3 => self.inputs.common_wall = (!text.trim().is_empty()).then_some(value),
            4 => self.inputs.pilot_z = value,
            5 => self.inputs.pilot_offset = value,
            10 => m.name = text,
            11 => m.thickness = value,
            12 => {
                if m.hole.min == m.hole.nominal && m.hole.max == m.hole.nominal {
                    m.hole = stackup::Band::exact(value);
                    self.drafts.remove(&(m.id, 13));
                    self.drafts.remove(&(m.id, 14));
                } else {
                    m.hole.nominal = value;
                }
            }
            13 => m.hole.min = value,
            14 => m.hole.max = value,
            15 => m.seating_min = value,
            16 => m.seating_max = value,
            17 => m.od_tolerance = value,
            18 => m.reference_normal_deg = value,
            19 => m.original.degrees = value,
            20 => m.finished.degrees = value,
            21 => m.original.zone_diameter = value,
            22 => m.finished.zone_diameter = value,
            23 => m.original.reference_span = value,
            24 => m.finished.reference_span = value,
            25 => m.wall_override = (!text.trim().is_empty()).then_some(value),
            26 => m.housing_material = text,
            27 => m.bushing_material = text,
            28 => m.finished_id_min = (!text.trim().is_empty()).then_some(value),
            30 => m.member_width = value,
            31 => m.member_depth = value,
            29 => m.finished_id_max = (!text.trim().is_empty()).then_some(value),
            _ => {}
        }
    }
    fn document(&self) -> Option<Document> {
        let m = &self.inputs.members[self.selected];
        let btn = |label: &str, a| Entry::new(label, "", action(a));
        let mut buttons = vec![
            btn("Recalculate", StackupAct::Recalculate),
            btn("Add", StackupAct::Add),
            btn("Remove", StackupAct::Remove).when(if self.inputs.members.len() > 1 {
                Ok(())
            } else {
                Err("Keep one member".into())
            }),
            btn("Up", StackupAct::Up).when(if self.selected > 0 {
                Ok(())
            } else {
                Err("Already first".into())
            }),
            btn("Down", StackupAct::Down).when(if self.selected + 1 < self.inputs.members.len() {
                Ok(())
            } else {
                Err("Already last".into())
            }),
            btn("Save", StackupAct::Save),
        ];
        buttons.push(btn("Send to Bushing", StackupAct::Send).when(self.transfer_availability()));
        let mut global = vec![
            (0, "Bushing ID nominal (in)"),
            (1, "Bushing ID min (in)"),
            (2, "Bushing ID max (in)"),
            (3, "Common wall default (in); blank disables"),
            (4, "Pilot position from stack entrance (in)"),
            (5, "Pilot center offset limit (in, radial)"),
        ];
        if !self.inputs.bushing_enabled {
            global.retain(|(id, _)| *id >= 4);
        }
        let mut member = vec![
            (10, "Member name"),
            (11, "Thickness along stack z (in)"),
            (12, "Current hole nominal (in)"),
            (13, "Current hole min (in)"),
            (14, "Current hole max (in)"),
            (30, "Member width (in)"),
            (31, "Member depth (in)"),
            (15, "Machining diameter minimum (in)"),
            (16, "Machining diameter maximum (in)"),
            (17, "Final hole tolerance + (in)"),
            (18, "Datum normal angle (deg)"),
            (25, "Wall override (in); blank inherits"),
            (26, "Housing material ID"),
            (27, "Bushing material ID"),
            (28, "Member finished ID lower limit (in, optional)"),
            (29, "Member finished ID upper limit (in, optional)"),
        ];
        match self.inputs.mode {
            AngularityMode::Degrees => member.extend([
                (19, "Original hole angularity ± (deg)"),
                (20, "Finished hole angularity ± (deg)"),
            ]),
            AngularityMode::CylindricalZone => member.extend([
                (21, "Original orientation zone diameter (in)"),
                (22, "Finished orientation zone diameter (in)"),
                (23, "Original zone span along datum normal (in)"),
                (24, "Finished zone span along datum normal (in)"),
            ]),
        }
        let panel = |label: &str, fields: Vec<(u8, &str)>| DocPanel {
            label: label.into(),
            badge: None,
            inputs: fields
                .into_iter()
                .map(|(id, label)| self.field(id, label))
                .collect(),
            cards: vec![],
        };
        let mut doc = Document {
            buttons,
            panels: vec![
                panel("Stack configuration", global),
                panel("Selected member", member),
            ],
            chips: vec![
                btn("Optional bushing", StackupAct::ToggleBushing)
                    .checked(self.inputs.bushing_enabled),
                btn("Uniform OD", StackupAct::Common).checked(self.inputs.uniform_od),
                btn(&format!("Mode {:?}", self.inputs.mode), StackupAct::Mode),
                btn(&format!("Wall {:?}", m.wall_mode), StackupAct::WallMode),
                btn("Use imported policy", StackupAct::AdoptImported).when(
                    if m.imported_wall.is_some() {
                        Ok(())
                    } else {
                        Err("No imported value".into())
                    },
                ),
            ],
            ..Document::default()
        };
        doc.precision = Some(PrecisionPage {
            inputs: self.inputs.clone(),
            result: self.result.as_ref().ok().cloned(),
            selected: self.selected,
            current: self.current(),
            error: self
                .current()
                .then(|| self.result.as_ref().err().cloned())
                .flatten(),
        });
        for mode in [WallMode::Required, WallMode::Informational] {
            doc.chips.push(
                btn(
                    &format!("Use imported as {mode:?}"),
                    StackupAct::ClassifyImported(mode),
                )
                .when(if m.imported_wall.is_some() {
                    Ok(())
                } else {
                    Err("No imported value".into())
                }),
            );
        }
        doc.notice=Some(DocNotice{tone:if self.current()&&self.result.is_ok(){Tone::Normal}else{Tone::Warn},text:format!("{} Input revision {} / result {:?}. Coplanar x-z, axial thickness; RFS straight-axis orientation zones. Machining and bushing axes share the common finished axis. No bonus tolerance or datum mobility.",if self.current(){"Current"}else{"STALE"},self.input_revision,self.result_revision),actions:vec![]});
        doc.policy = Some(DocTable {
            title: "Wall policy summary (calculation pending or invalid)".into(),
            columns: columns(
                &[
                    "Member/order",
                    "Effective",
                    "Source",
                    "Mode",
                    "Calculated min",
                    "Status",
                ],
                130.0,
            ),
            rows: self
                .inputs
                .members
                .iter()
                .enumerate()
                .map(|(index, member)| {
                    let p = stackup::wall_policy(&self.inputs, member);
                    DocRow {
                        cells: vec![
                            format!("{} / #{} {}", index + 1, member.id, member.name),
                            optional(p.effective),
                            p.source.into(),
                            format!("{:?}", p.mode),
                            "Not current".into(),
                            "Recalculate / review inputs".into(),
                        ],
                        sub: String::new(),
                        selected: index == self.selected,
                        select: action(StackupAct::Select(index)),
                        open: None,
                        badge: None,
                    }
                })
                .collect(),
            ..DocTable::default()
        });
        if !self.current() {
            doc.table.empty_title = "Results stale".into();
            doc.table.empty_body="Press Enter or leave the input field to update. Previous geometry is hidden while editing.".into();
            return Some(doc);
        }
        let Ok(r) = &self.result else {
            doc.table.empty_title = "No accepted calculation".into();
            doc.table.empty_body = self.result.as_ref().err().cloned().unwrap_or_default();
            return Some(doc);
        };
        let mut policy = DocTable {
            title: format!(
                "Wall policy summary{}",
                if self.current() { "" } else { " (STALE)" }
            ),
            columns: columns(
                &[
                    "Member/order",
                    "Effective",
                    "Source",
                    "Mode",
                    "Calculated min",
                    "Status",
                ],
                130.0,
            ),
            ..DocTable::default()
        };
        doc.table = DocTable {
            title: "Per-member sizing, inches; full precision calculated before display".into(),
            columns: columns(
                &[
                    "Layer / ID",
                    "Thickness",
                    "Original nominal [min,max]",
                    "Bushing ID [min,max]",
                    "Final cleaned member hole [min,max]",
                    "Angularity-only cleanup diameter",
                    "Enlargement reason",
                    "Finished bore capacity max",
                    "Original angle max deg",
                    "Finished angle max deg",
                    "Mode",
                    "Eccentric max / end offsets",
                    "Required wall",
                    "Informational reference",
                    "Min wall",
                    "Max directional wall",
                    "Wall mode/source",
                    "Status",
                    "Governing",
                ],
                170.0,
            ),
            ..DocTable::default()
        };
        for (index, (lr, member)) in r.layers.iter().zip(&self.inputs.members).enumerate() {
            let row = |cells| DocRow {
                cells,
                sub: String::new(),
                selected: index == self.selected,
                select: action(StackupAct::Select(index)),
                open: None,
                badge: None,
            };
            let state = if self.current() {
                format!("{:?}: {}", lr.status, lr.comparison)
            } else {
                "STALE".into()
            };
            policy.rows.push(row(vec![
                format!("{} / #{} {}", index + 1, member.id, member.name),
                optional(lr.policy.effective),
                lr.policy.source.into(),
                format!("{:?}", lr.policy.mode),
                format!("{:.5}", lr.wall_min),
                state.clone(),
            ]));
            let imported = lr
                .policy
                .imported
                .as_ref()
                .map(|w| {
                    format!(
                        "{:.5} {} {:?} {} / {}",
                        w.value, w.units, w.semantics, w.workbench, w.source_id
                    )
                })
                .unwrap_or_else(|| "None".into());
            let imported = format!(
                "{}; {}",
                imported,
                lr.imported_comparison.as_deref().unwrap_or("")
            );
            let band = |d: stackup::Band| format!("{:.5} [{:.5},{:.5}]", d.nominal, d.min, d.max);
            doc.table.rows.push(row(vec![
                format!("{} / #{} {}", index + 1, member.id, member.name),
                format!("{:.5}", member.thickness),
                band(member.hole),
                band(self.inputs.finished_id),
                band(lr.od),
                format!("{:.5}", lr.bore_capacity),
                format!(
                    "{:.5} deg / {:.5} in zone at L={:.5} in / dx={:.5} in over axial t={:.5} in",
                    lr.original_angle_max,
                    lr.original_zone_used,
                    member.original.reference_span,
                    lr.original_displacement,
                    member.thickness
                ),
                format!(
                    "{:.5} deg / {:.5} in zone at L={:.5} in / dx={:.5} in over axial t={:.5} in",
                    lr.finished_angle_max,
                    lr.finished_zone_used,
                    member.finished.reference_span,
                    lr.finished_displacement,
                    member.thickness
                ),
                format!("{:?}", self.inputs.mode),
                format!(
                    "{:.5} / {:.5},{:.5} (+x 0deg / -x 180deg)",
                    lr.eccentricity_max, lr.nominal_eccentricity[0], lr.nominal_eccentricity[1]
                ),
                if lr.policy.mode == WallMode::Required {
                    optional(lr.policy.effective)
                } else {
                    "Not mandatory".into()
                },
                format!(
                    "{}; imported {}",
                    if lr.policy.mode == WallMode::Informational {
                        optional(lr.policy.effective)
                    } else {
                        "None".into()
                    },
                    imported
                ),
                format!("{:.5}", lr.wall_min),
                format!("{:.5}", lr.wall_max),
                format!("{:?} / {}", lr.policy.mode, lr.policy.source),
                state,
                lr.governing.clone(),
            ]));
        }
        doc.policy = Some(policy);
        doc.detail=Some(DocDetail{drawing:Some(self.drawing(r)),title:"Stack section, x-z".into(),subtitle:"Shared limiting scenario, true scale. Orange original, green bore, blue common axis. Click a layer to edit.".into(),actions:vec![],blocks:vec![],empty:format!("Selected {} #{}. OD center x={:.5} in. Resulting bore capacity is the largest common circular reamer diameter allowed locally by selected OD and wall; actual requested ID remains shared. {}",m.name,m.id,r.layers[self.selected].center_x,if self.current(){""}else{"DRAWING STALE"})});
        Some(doc)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> StackupSession {
        StackupSession::open(Path::new("/tmp/never-created-stackup-v2.json")).unwrap()
    }
    #[test]
    fn exhausted_member_ids_do_not_mutate_stack() {
        let mut s = session();
        s.inputs.members[0].id = u64::MAX;
        let before = s.inputs.clone();
        s.act_stackup(StackupAct::Add);
        assert_eq!(s.inputs, before);
        assert!(s.status.contains("exhausted"));
    }
    #[test]
    fn window_case_lock_is_exclusive_and_releases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("case.json");
        let first = WindowLease::acquire(&path).unwrap();
        assert!(WindowLease::acquire(&path).is_err());
        drop(first);
        assert!(WindowLease::acquire(&path).is_ok());
    }
    #[test]
    fn recovery_preserves_invalid_drafts_and_member_selection() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = StackupSession::open(&dir.path().join("case.json")).unwrap();
        s.act_stackup(StackupAct::Select(1));
        s.document_field(11, "unfinished".into());
        std::fs::write(
            s.path.with_extension("recovery.json"),
            s.recovery_json().unwrap(),
        )
        .unwrap();
        let mut restored = StackupSession::open(&s.path).unwrap();
        restored.restore_recovery().unwrap();
        assert_eq!(restored.selected, 1);
        assert_eq!(restored.value(11), "unfinished");
        assert!(restored.dirty() && restored.result.is_err());
        assert_eq!(restored.document().unwrap().policy.unwrap().rows.len(), 2);
    }
    #[test]
    fn current_hole_edit_updates_exact_band_and_reports_bad_limits() {
        let mut s = session();
        s.document_field(12, "0.4".into());
        s.recalculate();
        assert_eq!(s.inputs.members[0].hole, stackup::Band::exact(0.4));
        assert!(s.result.is_ok());
        s.document_field(13, "0.5".into());
        s.recalculate();
        let page = s.document().unwrap().precision.unwrap();
        assert!(page.error.as_deref().unwrap().contains("dimensions"));
    }
    #[test]
    fn inactive_bushing_drafts_do_not_block_hole_only() {
        let mut s = session();
        s.act_stackup(StackupAct::ToggleBushing);
        s.document_field(0, "unfinished".into());
        s.act_stackup(StackupAct::ToggleBushing);
        s.recalculate();
        assert!(s.result.is_ok());
        s.act_stackup(StackupAct::ToggleBushing);
        s.recalculate();
        assert!(s.result.is_err());
    }
    #[test]
    fn invalid_text_invalidates_and_cannot_transfer() {
        let mut s = session();
        s.document_field(11, "bad".into());
        assert!(!s.current());
        s.recalculate();
        assert!(s.result.is_err());
        assert!(s.transfer_availability().is_err());
    }
    #[test]
    fn stable_identity_reorder_and_cache() {
        let mut s = session();
        let id = s.inputs.members[0].id;
        s.act_stackup(StackupAct::Down);
        assert_eq!(s.inputs.members[1].id, id);
        assert!(s.current());
        let revision = s.result_revision;
        assert_eq!(s.document().unwrap().table.rows.len(), 2);
        s.document();
        assert_eq!(s.result_revision, revision);
    }
    #[test]
    fn imported_classification_is_explicit_and_preserves_source() {
        let mut s = session();
        s.inputs.bushing_enabled = true;
        s.inputs.members[0].imported_wall = Some(stackup::ImportedWall {
            value: 0.1,
            units: "in".into(),
            workbench: "old".into(),
            source_id: "one".into(),
            semantics: WallMode::Unclassified,
            context: String::new(),
        });
        s.changed();
        s.recalculate();
        s.act_stackup(StackupAct::WallMode);
        s.recalculate();
        assert_eq!(
            s.result.as_ref().unwrap().layers[0].status,
            Status::ReviewRequired
        );
        s.act_stackup(StackupAct::ClassifyImported(WallMode::Required));
        s.recalculate();
        assert_eq!(s.inputs.members[0].wall_override, Some(0.1));
        assert_eq!(
            s.result.as_ref().unwrap().layers[0].policy.source,
            "Imported value (explicit use)"
        );
        assert_eq!(
            s.inputs.members[0]
                .imported_wall
                .as_ref()
                .unwrap()
                .semantics,
            WallMode::Unclassified
        );
    }
    #[test]
    fn modes_keep_distinct_values_and_roundtrip() {
        let mut s = session();
        let degrees = s.inputs.members[0].original.degrees;
        s.act_stackup(StackupAct::Mode);
        assert_eq!(s.inputs.members[0].original.degrees, degrees);
        let raw = serde_json::to_string(&StackupFile {
            version: 3,
            inputs: s.inputs.clone(),
        })
        .unwrap();
        assert_eq!(
            serde_json::from_str::<StackupFile>(&raw).unwrap().inputs,
            s.inputs
        );
    }
}
