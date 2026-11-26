//! Unified thread strength and damage comparison. Mechanics live in fastened-joint-solver.
use super::{
    authoring::{Authoring, Card, Relevance},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods, Tone},
};
use crate::gpu_viewer::{
    bench::Bench,
    controller::UiAction,
    menu::{Entry, Item, Menu},
    ui::{CardRow, Compare, CompareRow, DockItem, Look, Plot, Series, ValueCard, Workbench, WorkbenchView},
    Scene,
};
use fastened_joint_solver::thread_capability::{
    self as strength, Analysis, Case, Family, Interface, StiMeasurement,
};
use fastened_joint_solver::thread_hybrid::{self, ElasticInputs, ElasticResult};
use mechanics_core::materials::{builtin_catalog, Material};
use crate::toolboxes::material_lookup::model::{catalog, filter_sort, Query};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod drawing;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    FemaleMinor,
    BoltMajor,
    BoltMinor,
    Engagement,
    BoltPitch,
    FemalePitch,
    NominalMajor,
    Tpi,
    ParentShear,
    BoltTensile,
    BoltShear,
    Load,
    OuterMajor,
    OuterFemaleMinor,
    InsertLength,
    StiMinor,
    StiPitch,
    InactiveEntryTurns,
}
impl Change {
    const ALL: [Self; 18] = [
        Self::FemaleMinor,
        Self::BoltMajor,
        Self::BoltMinor,
        Self::Engagement,
        Self::BoltPitch,
        Self::FemalePitch,
        Self::NominalMajor,
        Self::Tpi,
        Self::ParentShear,
        Self::BoltTensile,
        Self::BoltShear,
        Self::Load,
        Self::OuterMajor,
        Self::OuterFemaleMinor,
        Self::InsertLength,
        Self::StiMinor,
        Self::StiPitch,
        Self::InactiveEntryTurns,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::FemaleMinor => "Internal minor diameter (damage)",
            Self::BoltMajor => "Bolt major diameter (damage)",
            Self::BoltMinor => "Bolt root diameter (damage)",
            Self::Engagement => "Effective engagement (loss)",
            Self::BoltPitch => "Bolt pitch diameter",
            Self::FemalePitch => "Internal pitch diameter",
            Self::NominalMajor => "Nominal major diameter",
            Self::Tpi => "Threads per inch",
            Self::ParentShear => "Parent shear strength",
            Self::BoltTensile => "Bolt tensile strength",
            Self::BoltShear => "Bolt shear strength",
            Self::Load => "Axial load",
            Self::OuterMajor => "Insert external major diameter",
            Self::OuterFemaleMinor => "Parent minor diameter",
            Self::InsertLength => "External engagement length",
            Self::StiMinor => "STI tapped-hole minor diameter (damage)",
            Self::StiPitch => "STI tapped-hole pitch diameter (damage)",
            Self::InactiveEntryTurns => "Fully ineffective entry turns",
        }
    }
    fn unit(self) -> &'static str {
        match self {
            Self::Tpi => "TPI",
            Self::ParentShear | Self::BoltTensile | Self::BoltShear => "ksi",
            Self::Load => "lbf",
            Self::InactiveEntryTurns => "turns",
            _ => "in",
        }
    }
    fn available(self, f: Family) -> bool {
        match self {
            Self::OuterMajor | Self::OuterFemaleMinor | Self::InsertLength => f == Family::Keensert,
            Self::StiMinor | Self::StiPitch => f == Family::HelicalCoil,
            Self::InactiveEntryTurns => f == Family::Tapped,
            _ => true,
        }
    }
    fn current(self, c: &Case) -> f64 {
        match self {
            Self::FemaleMinor => c.inner.female_minor,
            Self::BoltMajor => c.inner.major,
            Self::NominalMajor => c.nominal_major,
            Self::BoltMinor => c.inner.minor,
            Self::Engagement => c.inner.engagement,
            Self::BoltPitch => c.inner.pitch,
            Self::FemalePitch => c.inner.female_pitch,
            Self::Tpi => c.tpi,
            Self::ParentShear => c.parent_shear_strength / 1000.0,
            Self::BoltTensile => c.bolt_tensile_strength / 1000.0,
            Self::BoltShear => c.bolt_shear_strength / 1000.0,
            Self::Load => c.axial_load,
            Self::OuterMajor => c.outer.map_or(0.0, |(_, o)| o.major),
            Self::OuterFemaleMinor => c.outer.map_or(0.0, |(_, o)| o.female_minor),
            Self::InsertLength => c.outer.map_or(0.0, |(_, o)| o.engagement),
            Self::StiMinor => c.sti.map_or(0.0, |s| s.minor),
            Self::StiPitch => c.sti.map_or(0.0, |s| s.pitch),
            Self::InactiveEntryTurns => 0.0,
        }
    }
    fn suggestion(self, c: &Case) -> f64 {
        let v = self.current(c);
        match self {
            Self::FemaleMinor | Self::OuterFemaleMinor | Self::StiMinor | Self::StiPitch => {
                v + 0.005
            }
            Self::BoltMajor
            | Self::BoltMinor
            | Self::BoltPitch
            | Self::Engagement
            | Self::OuterMajor
            | Self::InsertLength => v - 0.005,
            Self::InactiveEntryTurns => 1.0,
            _ => v,
        }
    }
    fn apply(self, c: &mut Case, v: f64) {
        if v == self.current(c) {
            return;
        }
        match self {
            Self::FemaleMinor => c.inner.female_minor = v,
            Self::BoltMajor => c.inner.major = v,
            Self::BoltMinor => c.inner.minor = v,
            Self::Engagement => c.inner.engagement = v,
            Self::BoltPitch => c.inner.pitch = v,
            Self::FemalePitch => c.inner.female_pitch = v,
            Self::NominalMajor => {
                c.nominal_major = v;
            }
            Self::Tpi => {
                c.tpi = v;
            }
            Self::ParentShear => c.parent_shear_strength = v * 1000.0,
            Self::BoltTensile => c.bolt_tensile_strength = v * 1000.0,
            Self::BoltShear => c.bolt_shear_strength = v * 1000.0,
            Self::Load => c.axial_load = v,
            Self::OuterMajor => {
                if let Some((_, o)) = &mut c.outer {
                    o.major = v;
                }
            }
            Self::OuterFemaleMinor => {
                if let Some((_, o)) = &mut c.outer {
                    o.female_minor = v;
                }
            }
            Self::InsertLength => {
                if let Some((_, o)) = &mut c.outer {
                    o.engagement = v;
                }
            }
            Self::StiMinor => {
                if let Some(s) = &mut c.sti {
                    s.minor = v;
                }
            }
            Self::StiPitch => {
                if let Some(s) = &mut c.sti {
                    s.pitch = v;
                }
            }
            Self::InactiveEntryTurns => c.inner.engagement -= v / c.tpi,
        }
        // A rating is for the exact qualified installed geometry and material.
        if c.family != Family::Tapped && self != Self::Load {
            c.assembly_rating = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Major,
    Tpi,
    Engagement,
    Load,
    BoltTensile,
    BoltShear,
    ParentShear,
    InsertShear,
    OuterMajor,
    OuterTpi,
    OuterLength,
    Rating,
    RatingSource,
    Proposed,
    StiMinor,
    StiPitch,
    ParentWall,
}
impl Field {
    const ALL: [Self; 17] = [
        Self::Major,
        Self::Tpi,
        Self::Engagement,
        Self::Load,
        Self::BoltTensile,
        Self::BoltShear,
        Self::ParentShear,
        Self::InsertShear,
        Self::OuterMajor,
        Self::OuterTpi,
        Self::OuterLength,
        Self::Rating,
        Self::RatingSource,
        Self::Proposed,
        Self::StiMinor,
        Self::StiPitch,
        Self::ParentWall,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Major => "Nominal major diameter",
            Self::Tpi => "Threads per inch",
            Self::Engagement => "Effective engagement",
            Self::Load => "Axial load",
            Self::BoltTensile => "Bolt tensile strength",
            Self::BoltShear => "Bolt shear strength",
            Self::ParentShear => "Parent shear strength",
            Self::InsertShear => "Insert shear strength",
            Self::OuterMajor => "External insert major",
            Self::OuterTpi => "External insert TPI",
            Self::OuterLength => "External engagement",
            Self::Rating => "Assembly pull-out rating",
            Self::RatingSource => "Rating source",
            Self::Proposed => "Proposed value",
            Self::StiMinor => "STI tapped minor",
            Self::StiPitch => "STI tapped pitch",
            Self::ParentWall => "Minimum radial wall",
        }
    }
    fn unit(self) -> &'static str {
        match self {
            Self::Tpi | Self::OuterTpi => " TPI",
            Self::BoltTensile | Self::BoltShear | Self::ParentShear | Self::InsertShear => " ksi",
            Self::Load | Self::Rating => " lbf",
            Self::RatingSource => "",
            _ => " in",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct File {
    version: u32,
    name: String,
    case: Case,
    change: Change,
    proposed: f64,
    #[serde(default)]
    rating_value: Option<f64>,
    #[serde(default)]
    rating_source: String,
    #[serde(default)]
    bolt_material: Option<String>,
    #[serde(default)]
    parent_material: Option<String>,
    #[serde(default)]
    insert_material: Option<String>,
    #[serde(default)]
    parent_wall: Option<f64>,
}
impl File {
    fn material(id: &str) -> Option<&'static Material> {
        builtin_catalog().find(|m| m.id == id)
    }
    fn suggested(change: Change, case: &Case) -> f64 {
        let proposed = change.suggestion(case);
        let mut trial = case.clone();
        change.apply(&mut trial, proposed);
        if strength::compute(&trial).is_ok() {
            proposed
        } else {
            change.current(case)
        }
    }
    fn new() -> Self {
        let bolt = Self::material("steel4340").unwrap();
        let parent = Self::material("al7075").unwrap();
        let case = Case {
            family: Family::Tapped,
            nominal_major: 0.25,
            tpi: 28.0,
            inner: Interface::basic(0.25, 28.0, 0.375),
            outer: None,
            sti: None,
            axial_load: 3000.0,
            bolt_tensile_strength: bolt.ftu_ksi * 1000.0,
            bolt_shear_strength: bolt.fsu_ksi * 1000.0,
            parent_shear_strength: parent.fsu_ksi * 1000.0,
            insert_shear_strength: None,
            assembly_rating: None,
        };
        Self {
            version: 1,
            name: "Thread case".into(),
            change: Change::FemaleMinor,
            proposed: Self::suggested(Change::FemaleMinor, &case),
            case,
            rating_value: None,
            rating_source: String::new(),
            bolt_material: Some(bolt.id.into()),
            parent_material: Some(parent.id.into()),
            insert_material: None,
            parent_wall: None,
        }
    }
    fn sync_rating(&mut self) {
        self.case.assembly_rating = self
            .rating_value
            .filter(|_| !self.rating_source.trim().is_empty())
            .map(|v| (v, self.rating_source.trim().to_string()));
    }
    fn clear_rating(&mut self) {
        self.rating_value = None;
        self.rating_source.clear();
        self.case.assembly_rating = None;
    }
    fn changed(&self) -> Case {
        let mut c = self.case.clone();
        self.change.apply(&mut c, self.proposed);
        c
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("unsupported thread case version {}", self.version));
        }
        if self.parent_wall.is_some_and(|v| !v.is_finite() || v <= 0.0) {
            return Err("minimum radial parent wall must be positive".into());
        }
        if self.change == Change::InactiveEntryTurns
            && (self.case.family != Family::Tapped || !self.proposed.is_finite()
                || self.proposed < 0.0 || self.proposed.fract() != 0.0)
        {
            return Err("ineffective entry turns must be a nonnegative whole number on a tapped thread".into());
        }
        if self
            .rating_value
            .is_some_and(|v| !v.is_finite() || v <= 0.0)
        {
            return Err("assembly rating must be positive".into());
        }
        let expected = self
            .rating_value
            .filter(|_| !self.rating_source.trim().is_empty())
            .map(|v| (v, self.rating_source.trim().to_string()));
        if self.case.assembly_rating != expected {
            return Err("assembly rating and source disagree".into());
        }
        if let Some(id) = &self.bolt_material {
            let m = Self::material(id).ok_or("bolt material is missing from library")?;
            if self.case.bolt_tensile_strength != m.ftu_ksi * 1000.0
                || self.case.bolt_shear_strength != m.fsu_ksi * 1000.0 {
                return Err("bolt strengths disagree with material library".into());
            }
        }
        if let Some(id) = &self.parent_material {
            let m = Self::material(id).ok_or("parent material is missing from library")?;
            if self.case.parent_shear_strength != m.fsu_ksi * 1000.0 {
                return Err("parent strength disagrees with material library".into());
            }
        }
        if let Some(id) = &self.insert_material {
            let m = Self::material(id).ok_or("insert material is missing from library")?;
            if self.case.family == Family::Keensert
                && self.case.insert_shear_strength != Some(m.fsu_ksi * 1000.0) {
                return Err("insert strength disagrees with material library".into());
            }
        }
        strength::compute(&self.case).map_err(str::to_string)?;
        strength::compute(&self.changed()).map_err(|e| format!("proposed change: {e}"))?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadAct {
    Family(Family),
    Change(Change),
    OpenChange,
    OpenMaterial(MaterialRole),
    ClosePicker,
    View(bool),
    Edit(usize),
    Material(MaterialRole, &'static str),
    Undo,
    Redo,
    Save,
    Report,
    Evidence,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialRole { Bolt, Parent, Insert }
enum Picker { Change, Material { role: MaterialRole, query: String } }
struct Prompt {
    field: Field,
    text: String,
    fresh: bool,
}
pub struct ThreadSession {
    file: File,
    path: PathBuf,
    revision: Option<String>,
    dirty: bool,
    undo: Vec<File>,
    redo: Vec<File>,
    prompt: Option<Prompt>,
    picker: Option<Picker>,
    status: String,
    changed_view: bool,
    drawn: bool,
    translucent_from: Option<u32>,
}
impl ThreadSession {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = if path.exists() {
            let s = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            serde_json::from_str::<File>(&s).map_err(|e| format!("not a thread case: {e}"))?
        } else {
            File::new()
        };
        file.validate()?;
        Ok(Self {
            file,
            path: path.to_owned(),
            revision: crate::persistence::revision(path),
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            picker: None,
            status: String::new(),
            changed_view: false,
            drawn: false,
            translucent_from: None,
        })
    }
    fn action(a: ThreadAct) -> UiAction {
        UiAction::Editor(EditorAction::Thread(a))
    }
    fn entry(label: &str, a: ThreadAct) -> Entry {
        Entry::new(label, "", Self::action(a))
    }
    fn checkpoint(&mut self) {
        self.undo.push(self.file.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn edited(&mut self) {
        self.dirty = true;
        self.drawn = false;
        self.status.clear();
    }
    fn set_family(&mut self, f: Family) {
        if self.file.case.family == f {
            return;
        }
        self.checkpoint();
        self.file.case.family = f;
        self.file.clear_rating();
        match f {
            Family::Tapped => {
                self.file.case.outer = None;
                self.file.case.sti = None;
                self.file.case.insert_shear_strength = None;
                self.file.insert_material = None;
            }
            Family::HelicalCoil => {
                self.file.case.outer = None;
                self.file.case.insert_shear_strength = None;
                self.file.insert_material = None;
                self.file.case.sti =
                    strength::sti_limits(self.file.case.inner.major, self.file.case.tpi).map(|l| {
                        StiMeasurement {
                            pitch: l.pitch_max,
                            minor: l.minor_max,
                        }
                    });
            }
            Family::Keensert => {
                self.file.case.outer = Some((16.0, Interface::basic(0.375, 16.0, 0.37)));
                self.file.case.sti = None;
                self.file.case.insert_shear_strength = None;
            }
        }
        if !self.file.change.available(f) {
            self.file.change = Change::FemaleMinor;
        }
        self.file.proposed = File::suggested(self.file.change, &self.file.case);
        self.picker = None;
        self.edited();
    }
    fn set_material(&mut self, role: MaterialRole, id: &'static str) {
        let Some(m) = File::material(id) else { return };
        if m.fsu_ksi <= 0.0 || (role == MaterialRole::Bolt && m.ftu_ksi <= 0.0) { return; }
        let mut next = self.file.clone();
        match role {
            MaterialRole::Bolt => {
                next.bolt_material = Some(id.into());
                next.case.bolt_tensile_strength = m.ftu_ksi * 1000.0;
                next.case.bolt_shear_strength = m.fsu_ksi * 1000.0;
            }
            MaterialRole::Parent => {
                next.parent_material = Some(id.into());
                next.case.parent_shear_strength = m.fsu_ksi * 1000.0;
            }
            MaterialRole::Insert => {
                next.insert_material = Some(id.into());
                if next.case.family == Family::Keensert {
                    next.case.insert_shear_strength = Some(m.fsu_ksi * 1000.0);
                }
            }
        }
        next.clear_rating();
        next.proposed = File::suggested(next.change, &next.case);
        if let Err(e) = next.validate() { self.status = e; return; }
        self.checkpoint();
        self.file = next;
        self.picker = None;
        self.edited();
    }
    fn set_change(&mut self, ch: Change) {
        if !ch.available(self.file.case.family) {
            return;
        }
        self.checkpoint();
        self.file.change = ch;
        self.file.proposed = File::suggested(ch, &self.file.case);
        self.picker = None;
        self.edited();
    }
    fn value(&self, f: Field) -> String {
        let c = &self.file.case;
        match f {
            Field::Major => format!("{:.4}", c.nominal_major),
            Field::Tpi => format!("{:.0}", c.tpi),
            Field::Engagement => format!("{:.4}", c.inner.engagement),
            Field::Load => format!("{:.0}", c.axial_load),
            Field::BoltTensile => format!("{:.1}", c.bolt_tensile_strength / 1000.0),
            Field::BoltShear => format!("{:.1}", c.bolt_shear_strength / 1000.0),
            Field::ParentShear => format!("{:.1}", c.parent_shear_strength / 1000.0),
            Field::InsertShear => c
                .insert_shear_strength
                .map_or("not supplied".into(), |v| format!("{:.1}", v / 1000.0)),
            Field::OuterMajor => c
                .outer
                .map_or("not supplied".into(), |(_, o)| format!("{:.4}", o.major)),
            Field::OuterTpi => c
                .outer
                .map_or("not supplied".into(), |(n, _)| format!("{n:.0}")),
            Field::OuterLength => c.outer.map_or("not supplied".into(), |(_, o)| {
                format!("{:.4}", o.engagement)
            }),
            Field::Rating => self
                .file
                .rating_value
                .map_or("not supplied".into(), |v| format!("{v:.0}")),
            Field::RatingSource => {
                if self.file.rating_source.is_empty() {
                    "not supplied".into()
                } else {
                    self.file.rating_source.clone()
                }
            }
            Field::Proposed => format!("{:.4}", self.file.proposed),
            Field::StiMinor => c
                .sti
                .map_or("not supplied".into(), |s| format!("{:.4}", s.minor)),
            Field::StiPitch => c
                .sti
                .map_or("not supplied".into(), |s| format!("{:.4}", s.pitch)),
            Field::ParentWall => self.file.parent_wall.map_or("not supplied".into(), |v| format!("{v:.4}")),
        }
    }
    fn edit(&mut self, f: Field) {
        self.picker = None;
        let v = self.value(f);
        self.prompt = Some(Prompt {
            field: f,
            text: if v == "not supplied" {
                String::new()
            } else {
                v
            },
            fresh: true,
        });
    }
    fn commit(&mut self) {
        let Some(p) = self.prompt.take() else {
            return;
        };
        let f = p.field;
        let mut next = self.file.clone();
        if f == Field::RatingSource {
            next.rating_source = p.text.trim().to_string();
        } else if f == Field::ParentWall && p.text.trim().is_empty() {
            next.parent_wall = None;
        } else {
            let Ok(v) = p.text.trim().parse::<f64>() else {
                self.status = format!("{} needs a number.", f.label());
                return;
            };
            if !v.is_finite() {
                self.status = format!("{} must be finite.", f.label());
                return;
            }
            match f {
                Field::Major => {
                    next.case.nominal_major = v;
                    next.case.inner =
                        Interface::basic(v, next.case.tpi, next.case.inner.engagement);
                }
                Field::Tpi => {
                    next.case.tpi = v;
                    next.case.inner =
                        Interface::basic(next.case.inner.major, v, next.case.inner.engagement);
                }
                Field::Engagement => next.case.inner.engagement = v,
                Field::Load => next.case.axial_load = v,
                Field::BoltTensile => { next.case.bolt_tensile_strength = v * 1000.0; next.bolt_material = None; }
                Field::BoltShear => { next.case.bolt_shear_strength = v * 1000.0; next.bolt_material = None; }
                Field::ParentShear => { next.case.parent_shear_strength = v * 1000.0; next.parent_material = None; }
                Field::InsertShear => { next.case.insert_shear_strength = Some(v * 1000.0); next.insert_material = None; }
                Field::OuterMajor => {
                    if let Some((n, o)) = &mut next.case.outer {
                        *o = Interface::basic(v, *n, o.engagement);
                    }
                }
                Field::OuterTpi => {
                    if let Some((n, o)) = &mut next.case.outer {
                        *n = v;
                        *o = Interface::basic(o.major, v, o.engagement);
                    }
                }
                Field::OuterLength => {
                    if let Some((_, o)) = &mut next.case.outer {
                        o.engagement = v;
                    }
                }
                Field::StiMinor => {
                    if let Some(s) = &mut next.case.sti {
                        s.minor = v;
                    }
                }
                Field::StiPitch => {
                    if let Some(s) = &mut next.case.sti {
                        s.pitch = v;
                    }
                }
                Field::Rating => next.rating_value = Some(v),
                Field::Proposed => next.proposed = v,
                Field::ParentWall => next.parent_wall = Some(v),
                Field::RatingSource => unreachable!(),
            }
        }
        if f != Field::Proposed
            && f != Field::Load
            && f != Field::Rating
            && f != Field::RatingSource
        {
            next.clear_rating();
        }
        if matches!(f, Field::Major | Field::Tpi) && next.case.family == Family::HelicalCoil {
            next.case.sti = strength::sti_limits(next.case.nominal_major, next.case.tpi).map(|l| {
                StiMeasurement {
                    pitch: l.pitch_max,
                    minor: l.minor_max,
                }
            });
        }
        if f == Field::Rating || f == Field::RatingSource {
            next.sync_rating();
        }
        if f != Field::Proposed {
            next.proposed = File::suggested(next.change, &next.case);
        }
        if let Err(e) = next.validate() {
            self.status = format!("{} unchanged: {e}", f.label());
            return;
        }
        self.checkpoint();
        self.file = next;
        self.edited();
    }
    fn undo(&mut self) {
        if let Some(x) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.file, x));
            self.edited();
        }
    }
    fn redo(&mut self) {
        if let Some(x) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.file, x));
            self.edited();
        }
    }
    fn save(&mut self) {
        if crate::persistence::revision(&self.path) != self.revision {
            self.status = "Save conflict: file changed externally; reopen before saving.".into();
            return;
        }
        match serde_json::to_string_pretty(&self.file)
            .map_err(|e| e.to_string())
            .and_then(|s| crate::persistence::write(&self.path, s).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.dirty = false;
                self.revision = crate::persistence::revision(&self.path);
                self.status = format!("Saved {}.", self.path.display());
            }
            Err(e) => self.status = format!("Could not save {}: {e}", self.path.display()),
        }
    }
    fn report(&mut self) {
        let path = self.path.with_extension("report.txt");
        match crate::persistence::write(&path, self.report_text()) {
            Ok(()) => self.status = format!("Report written to {}.", path.display()),
            Err(e) => self.status = format!("Could not write report: {e}"),
        }
    }
    fn run(&mut self, a: ThreadAct) {
        match a {
            ThreadAct::Family(f) => self.set_family(f),
            ThreadAct::Material(role, id) => self.set_material(role, id),
            ThreadAct::Change(c) => self.set_change(c),
            ThreadAct::OpenChange => self.picker = Some(Picker::Change),
            ThreadAct::OpenMaterial(role) => self.picker = Some(Picker::Material { role, query: String::new() }),
            ThreadAct::ClosePicker => self.picker = None,
            ThreadAct::View(v) => {
                self.changed_view = v;
                self.drawn = false;
            }
            ThreadAct::Edit(i) => {
                if let Some(f) = Field::ALL.get(i) {
                    self.edit(*f);
                }
            }
            ThreadAct::Undo => self.undo(),
            ThreadAct::Redo => self.redo(),
            ThreadAct::Save => self.save(),
            ThreadAct::Report => self.report(),
            ThreadAct::Evidence => {
                self.status = match super::thread_evidence::open() {
                    Ok(()) => "Evidence tables opened in the browser.".into(),
                    Err(e) => format!("Could not open evidence tables: {e}"),
                };
            }
        }
    }
    fn analyses(&self) -> (Analysis, Analysis) {
        (
            strength::compute(&self.file.case).expect("validated baseline"),
            strength::compute(&self.file.changed()).expect("validated proposal"),
        )
    }
    fn elastic_fe(&self) -> Result<(ElasticResult, ElasticResult), String> {
        if self.file.case.family != Family::Tapped {
            return Err("Elastic FE currently supports directly tapped threads only.".into());
        }
        let material = |id: &Option<String>, role: &str| {
            id.as_deref().and_then(File::material)
                .ok_or_else(|| format!("Select a catalog {role} material for elastic FE."))
        };
        let bolt = material(&self.file.bolt_material, "bolt")?;
        let parent = material(&self.file.parent_material, "parent")?;
        let c = &self.file.case;
        let wall = self.file.parent_wall.unwrap_or(0.5 * c.nominal_major);
        let input = ElasticInputs {
            bolt_e_psi: bolt.e_ksi * 1000.0,
            bolt_nu: bolt.nu,
            parent_e_psi: parent.e_ksi * 1000.0,
            parent_nu: parent.nu,
            parent_outer_diameter_in: c.nominal_major + 2.0 * wall,
        };
        let baseline = thread_hybrid::tapped_load_share(c, input).map_err(str::to_string)?;
        let changed = thread_hybrid::tapped_load_share(&self.file.changed(), input).map_err(str::to_string)?;
        Ok((baseline, changed))
    }
    fn row(&self, f: Field) -> DockItem {
        let editing = self.prompt.as_ref().filter(|p| p.field == f);
        let value = editing.map_or_else(|| self.value(f), |p| p.text.clone());
        let unit = if editing.is_some() || (f == Field::ParentWall && self.file.parent_wall.is_none()) {
            String::new()
        } else if f == Field::Proposed {
            format!(" {}", self.file.change.unit())
        } else {
            f.unit().into()
        };
        DockItem::field(
            f.label(),
            format!(
                "{}{}{}",
                value,
                if editing.is_some() { "_" } else { "" },
                unit
            ),
            if editing.is_some() {
                Tone::Warn
            } else {
                Tone::Normal
            },
            Some(Self::action(ThreadAct::Edit(
                Field::ALL.iter().position(|x| *x == f).unwrap(),
            ))),
            editing.is_some(),
        )
    }
    fn material_row(&self, role: MaterialRole) -> DockItem {
        let selected = match role {
            MaterialRole::Bolt => &self.file.bolt_material,
            MaterialRole::Parent => &self.file.parent_material,
            MaterialRole::Insert => &self.file.insert_material,
        };
        let label = match role { MaterialRole::Bolt => "Bolt material", MaterialRole::Parent => "Parent material", MaterialRole::Insert => "Insert material" };
        DockItem::field(label, selected.as_deref().and_then(File::material).map_or("Search library…", |m| m.name), Tone::Normal, Some(Self::action(ThreadAct::OpenMaterial(role))), false)
    }
    fn inputs(&self) -> Vec<Card> {
        let c = &self.file.case;
        let family = Card::new(
            "thread-family",
            "Connection",
            Relevance::Primary,
            vec![
                DockItem::Segmented(
                    [
                        (Family::Tapped, "Tapped"),
                        (Family::HelicalCoil, "Helical coil"),
                        (Family::Keensert, "Keensert"),
                    ]
                    .into_iter()
                    .map(|(f, l)| Self::entry(l, ThreadAct::Family(f)).checked(c.family == f))
                    .collect(),
                ),
                DockItem::text(
                    "Unified 60° profile. Basic dimensions do not certify a thread class.",
                    Tone::Dim,
                ),
            ],
        );
        let geometry = Card::new(
            "thread-geometry",
            "Baseline geometry",
            Relevance::Primary,
            vec![
                self.row(Field::Major),
                self.row(Field::Tpi),
                self.row(Field::Engagement),
                self.row(Field::ParentWall),
                DockItem::text("Radial wall is measured from the outermost parent thread root to the nearest free surface.", Tone::Dim),
                DockItem::field(
                    "Basic pitch diameter",
                    format!("{:.4} in", c.inner.pitch),
                    Tone::Dim,
                    None,
                    false,
                ),
                DockItem::field(
                    "Basic minor diameter",
                    format!("{:.4} in", c.inner.female_minor),
                    Tone::Dim,
                    None,
                    false,
                ),
            ],
        )
        .tag("ASME B1.1");
        let mut load_rows = vec![self.material_row(MaterialRole::Bolt), self.material_row(MaterialRole::Parent)];
        load_rows.extend([self.row(Field::Load),self.row(Field::BoltTensile),self.row(Field::BoltShear), self.row(Field::ParentShear),
            DockItem::text("Catalog values are material strengths, not certified fastener ratings. Check product form, condition and basis.",Tone::Warn),
            DockItem::text("Results are an axial screen, not design allowables.",Tone::Dim)]);
        let load=Card::new("thread-load","Load and strengths",Relevance::Primary,load_rows);
        let mut cards = vec![family, geometry, load];
        if c.family == Family::Tapped {
            cards.push(Card::new("thread-tap", "Hole preparation", Relevance::Primary, vec![
                DockItem::text("Regular Unified tap matches the bolt thread. An STI tap is not interchangeable.", Tone::Dim),
                DockItem::text("Pilot drill and tap class require the selected process specification.", Tone::Dim),
            ]));
        }
        if c.family != Family::Tapped {
            let mut rows = vec![DockItem::text(
                if c.family == Family::HelicalCoil {
                    "Select a matching Heli-Coil STI assembly rating for this installed case."
                } else {
                    "External thread dimensions and key/body rating must match the selected Keensert."
                },
                Tone::Dim,
            )];
            if c.family == Family::Keensert {
                rows.push(DockItem::text("Regular Unified tap matches the insert external thread; do not use an STI tap.", Tone::Dim));
                if let Some((outer_tpi, outer)) = c.outer {
                    if let Some(i) = strength::keensert_installation((c.nominal_major, c.tpi), (outer.major, outer_tpi), outer.engagement) {
                        rows.push(DockItem::text(format!("Howmet carbon part {} geometry: regular tap drill {:.4} in; countersink {:.3} in; tap depth ≥{:.3} in", i.carbon_part, i.regular_tap_drill, i.countersink, i.min_tap_depth), Tone::Dim));
                    }
                }
                rows.push(self.material_row(MaterialRole::Insert));
                rows.push(DockItem::text("Select only the actual insert alloy and condition; a matching Howmet geometry does not identify its strength.", Tone::Warn));
                rows.extend([
                    self.row(Field::InsertShear),
                    self.row(Field::OuterMajor),
                    self.row(Field::OuterTpi),
                    self.row(Field::OuterLength),
                ]);
            }
            if c.family == Family::HelicalCoil {
                rows.push(DockItem::text("Standard Heli-Coil wire: cold-worked 304 stainless, tensile 200–250 ksi by wire size; no matched wire shear rating.", Tone::Dim));
                if let Some(installation) = strength::sti_installation(c.nominal_major, c.tpi) {
                    let l = installation.limits;
                    rows.push(DockItem::text("Use an STI tap for the parent hole. The bolt uses a regular Unified thread.", Tone::Dim));
                    rows.push(DockItem::text(format!(
                        "Suggested STI pilot drill: aluminum {:.4} in; steel, magnesium or plastic {:.4} in",
                        installation.drill_aluminum, installation.drill_steel_magnesium_plastic
                    ), Tone::Dim));
                    rows.push(DockItem::text(
                        format!(
                            "Stanley 2B STI limits: pitch {:.4}–{:.4}, minor {:.4}–{:.4} in",
                            l.pitch_min, l.pitch_max, l.minor_min, l.minor_max
                        ),
                        Tone::Dim,
                    ));
                    rows.extend([self.row(Field::StiPitch), self.row(Field::StiMinor)]);
                } else {
                    rows.push(DockItem::text(
                        "No sourced STI limits for this size; assembly capacity unavailable.",
                        Tone::Warn,
                    ));
                }
            }
            rows.extend([self.row(Field::Rating), self.row(Field::RatingSource)]);
            cards.push(
                Card::new("thread-insert", "Insert data", Relevance::Primary, rows)
                    .tag("manufacturer source"),
            );
        }
        let study=vec![
            DockItem::field("Characteristic",self.file.change.label(),Tone::Normal,
                Some(Self::action(ThreadAct::OpenChange)),false),
            DockItem::field("Baseline value",format!("{:.4} {}",self.file.change.current(c),self.file.change.unit()),Tone::Dim,None,false),
            self.row(Field::Proposed),
            if self.file.change == Change::InactiveEntryTurns {
                DockItem::text("Whole entry turns are removed from idealized engagement; local load redistribution is not modeled.", Tone::Dim)
            } else {
                DockItem::text("One measured characteristic changes.", Tone::Dim)
            },
            DockItem::text("Parent wall defines an idealized uniform FE annulus; no wall correction changes capacity.",Tone::Dim),
            DockItem::text("One characteristic changes. An insert rating is withdrawn when geometry or material changes.",Tone::Dim),
            DockItem::text("Click a value, type, Enter applies, Esc cancels.",Tone::Dim)];
        cards.push(Card::new(
            "thread-study",
            "Damage / change study",
            Relevance::Primary,
            study,
        ));
        cards
    }
    fn results(&self) -> Vec<Card> {
        let (base, changed) = self.analyses();
        let mut summary = vec![
            DockItem::text(
                match &changed.governing {
                    Some(g) => format!(
                        "Lowest modeled axial capacity: {} · {:.0} lbf",
                        g.name, g.lbf
                    ),
                    None => "Assembly capacity incomplete: required paths lack data.".into(),
                },
                if changed.governing.is_some() {
                    Tone::On
                } else {
                    Tone::Warn
                },
            ),
            DockItem::field(
                "Applied axial load",
                format!("{:.0} lbf", changed.axial_load),
                Tone::Normal,
                None,
                false,
            ),
        ];
        if let Some(g) = &changed.governing {
            if changed.axial_load > 0.0 {
                summary.push(DockItem::field(
                    "Capacity / load",
                    format!("{:.2}", g.lbf / changed.axial_load),
                    Tone::Normal,
                    None,
                    false,
                ));
            }
        }
        summary.push(DockItem::text(
            "No safety factor, preload, fatigue, temperature or transverse load is included.",
            Tone::Dim,
        ));
        let mut compare = Compare {
            heads: vec![
                "Failure path".into(),
                "Baseline lbf".into(),
                "Changed lbf".into(),
            ],
            rows: Vec::new(),
            footer: format!(
                "{}: {:.4} {} to {:.4} {}",
                self.file.change.label(),
                self.file.change.current(&self.file.case),
                self.file.change.unit(),
                self.file.proposed,
                self.file.change.unit()
            ),
        };
        for row in &base.rows {
            let new = changed.rows.iter().find(|r| r.name == row.name);
            compare.rows.push(CompareRow {
                title: row.name.into(),
                detail: row.basis.into(),
                values: vec![
                    format!("{:.0}", row.lbf),
                    new.map_or("not rated".into(), |r| format!("{:.0}", r.lbf)),
                ],
                governing: changed
                    .governing
                    .as_ref()
                    .is_some_and(|g| g.name == row.name),
                chip: new.map(|r| {
                    (
                        format!("{:+.1}%", 100.0 * (r.lbf / row.lbf - 1.0)),
                        if r.lbf < row.lbf {
                            Tone::Warn
                        } else {
                            Tone::Dim
                        },
                    )
                }),
                ..Default::default()
            });
        }
        for row in &changed.rows {
            if !base.rows.iter().any(|r| r.name == row.name) {
                compare.rows.push(CompareRow {
                    title: row.name.into(),
                    detail: row.basis.into(),
                    values: vec!["not rated".into(), format!("{:.0}", row.lbf)],
                    ..Default::default()
                });
            }
        }
        let mut limits = changed
            .missing
            .iter()
            .map(|m| DockItem::text(format!("Needs: {m}"), Tone::Warn))
            .collect::<Vec<_>>();
        if limits.is_empty() {
            limits.push(DockItem::text(
                "All modeled axial paths have numeric data. Confirm source applicability.",
                Tone::Dim,
            ));
        }
        limits.push(DockItem::text("Measured damage replaces one basic dimension. Class limits and damage are never guessed.",Tone::Dim));
        if let Some(wall) = self.file.parent_wall {
            limits.push(DockItem::text(format!("Minimum radial wall: {wall:.4} in (recorded; no capacity adjustment)."), Tone::Dim));
        }
        let mut cards = vec![
            Card::new(
                "thread-summary",
                "Strength capability",
                Relevance::Info,
                summary,
            ),
            Card::new(
                "thread-compare",
                "Baseline vs change",
                Relevance::Info,
                vec![DockItem::Compare(Box::new(compare))],
            ),
            Card::new(
                "thread-limits",
                "Missing evidence and limits",
                Relevance::Info,
                limits,
            ),
        ];
        let mut fe = Vec::new();
        match self.elastic_fe() {
            Ok((base, changed)) => {
                let changed_load = self.file.changed().axial_load;
                fe.push(DockItem::field("Peak turn share, baseline / changed",
                    format!("{:.1}% / {:.1}%", 100.0 * base.peak_fraction, 100.0 * changed.peak_fraction),
                    Tone::Normal, None, false));
                let points = |result: &ElasticResult, total: f64| result.per_turn_lbf.iter().enumerate()
                    .map(|(i, &load)| [(i + 1) as f64, 100.0 * load / total])
                    .collect();
                fe.push(DockItem::Plot(Plot {
                    x_label: "Engaged turn from loaded face".into(),
                    y_label: "Axial load share, %".into(),
                    series: vec![
                        Series { label: "Baseline".into(), points: points(&base, self.file.case.axial_load), color: 0, dashed: false },
                        Series { label: "Changed".into(), points: points(&changed, changed_load), color: 1, dashed: true },
                    ],
                    ..Plot::default()
                }));
                let wall = self.file.parent_wall.map_or_else(
                    || format!("assumed {:.4} in (0.5D)", 0.5 * self.file.case.nominal_major),
                    |v| format!("entered minimum {v:.4} in"));
                fe.push(DockItem::text(format!("1D elastic bolt/parent FE; {wall} modeled uniformly around a cylindrical boss."), Tone::Dim));
                if self.file.change.current(&self.file.case) != self.file.proposed
                    && base.per_turn_lbf.len() == changed.per_turn_lbf.len()
                    && base.per_turn_lbf.iter().zip(&changed.per_turn_lbf).all(|(&before, &after)|
                        (before / self.file.case.axial_load - after / changed_load).abs() < 1e-10) {
                    fe.push(DockItem::text("Selected change does not alter elastic turn sharing in this model; plotted curves overlap.", Tone::Dim));
                }
                fe.push(DockItem::text("Load sharing only: no flank clearance, plasticity, rupture, coil or Keensert behavior. Analytical capacities are unchanged.", Tone::Dim));
            }
            Err(reason) => fe.push(DockItem::text(reason, Tone::Warn)),
        }
        cards.insert(1, Card::new("thread-elastic-fe", "Elastic FE load sharing", Relevance::Info, fe));
        cards
    }
    fn workbench_data(&self) -> Workbench {
        let (_, changed) = self.analyses();
        Workbench {
            chips: vec![
                ("Unified · in · lbf · ksi".into(), Tone::Normal),
                (
                    if changed.governing.is_some() {
                        "Axial screen complete"
                    } else {
                        "Assembly evidence needed"
                    }
                    .into(),
                    if changed.governing.is_some() {
                        Tone::On
                    } else {
                        Tone::Warn
                    },
                ),
            ],
            actions: vec![
                Self::entry("Save case", ThreadAct::Save),
                Self::entry("Report", ThreadAct::Report),
                Self::entry("Evidence tables", ThreadAct::Evidence),
            ],
            inputs: self.inputs(),
            inputs_note: "Click a value to type; Enter applies.".into(),
            views: vec![
                Self::entry("Baseline", ThreadAct::View(false)).checked(!self.changed_view),
                Self::entry("Changed", ThreadAct::View(true)).checked(self.changed_view),
            ],
            show: WorkbenchView::Model,
            fields: Vec::new(),
            switches: Vec::new(),
            view_note: String::new(),
            model_title: format!(
                "{:.4}-{:.0} Unified · {}",
                self.file.case.inner.major,
                self.file.case.tpi,
                if self.changed_view {
                    "changed"
                } else {
                    "baseline"
                }
            ),
            model_hint: "Illustrative assembled threads; clearances are enlarged for visibility. Strength values are in the comparison table."
                .into(),
            sketches: Vec::new(),
            results: self.results(),
            dialog: None,
            look: Look::Boxed,
            under: Vec::new(),
            view_status: None,
        }
    }
    fn report_text(&self) -> String {
        let (base, changed) = self.analyses();
        let mut s=format!("Thread Workbench · {}\nUnified 60-degree thread · {:.4}-{:.0}\nFamily: {:?}\nChange: {} from {:.4} {} to {:.4} {}\nAxial load: {:.0} lbf\n\n",
            self.file.name,self.file.case.inner.major,self.file.case.tpi,self.file.case.family,self.file.change.label(),
            self.file.change.current(&self.file.case),self.file.change.unit(),self.file.proposed,self.file.change.unit(),changed.axial_load);
        for (name, id) in [("Bolt", &self.file.bolt_material), ("Parent", &self.file.parent_material), ("Insert", &self.file.insert_material)] {
            if name == "Insert" && self.file.case.family == Family::Tapped { continue; }
            let material = id.as_deref().and_then(File::material).map_or("manual / unspecified", |m| m.name);
            s.push_str(&format!("{name} material: {material}\n"));
        }
        if let Some(wall) = self.file.parent_wall {
            s.push_str(&format!("Minimum radial wall: {wall:.4} in; no wall correction applied\n"));
        }
        if self.file.change == Change::InactiveEntryTurns {
            s.push_str("Entry-turn loss assumes fully ineffective turns and removes their pitch length from uniform engagement. It does not model local load redistribution.\n");
        }
        match self.file.case.family {
            Family::Tapped => s.push_str("Hole tap: regular Unified, matching bolt thread; pilot drill not specified\n"),
            Family::HelicalCoil => {
                s.push_str("Hole tap: STI, matching wire insert\n");
                if let Some(i) = strength::sti_installation(self.file.case.nominal_major, self.file.case.tpi) {
                    s.push_str(&format!("Suggested STI pilot drill: aluminum {:.4} in; steel/magnesium/plastic {:.4} in (Heli-Coil Bulletin 2003 Table V)\n", i.drill_aluminum, i.drill_steel_magnesium_plastic));
                }
            }
            Family::Keensert => {
                s.push_str("Hole tap: regular Unified, matching insert external thread\n");
                if let Some((n, o)) = self.file.case.outer {
                    if let Some(i) = strength::keensert_installation((self.file.case.nominal_major, self.file.case.tpi), (o.major, n), o.engagement) {
                        s.push_str(&format!("Howmet carbon part {} geometry: pilot drill {:.4} in; minimum tap depth {:.3} in\n", i.carbon_part, i.regular_tap_drill, i.min_tap_depth));
                    }
                }
            }
        }
        s.push('\n');
        for r in &base.rows {
            let after = changed
                .rows
                .iter()
                .find(|x| x.name == r.name)
                .map_or("not rated".into(), |x| format!("{:.1}", x.lbf));
            s.push_str(&format!(
                "{}: {:.1} -> {} lbf ({})\n",
                r.name, r.lbf, after, r.basis
            ));
        }
        for m in &changed.missing {
            s.push_str(&format!("Missing: {m}\n"));
        }
        if let Some(g) = &base.governing {
            s.push_str(&format!(
                "Baseline governing: {} at {:.1} lbf\n",
                g.name, g.lbf
            ));
        }
        if let Some(g) = &changed.governing {
            s.push_str(&format!(
                "Changed governing: {} at {:.1} lbf\n",
                g.name, g.lbf
            ));
        } else {
            s.push_str("Changed governing assembly capacity: unavailable; missing evidence listed above.\n");
        }
        if let Some((n, source)) = &self.file.case.assembly_rating {
            s.push_str(&format!(
                "Manufacturer assembly rating: {n:.0} lbf; source: {source}\n"
            ));
        }
        match self.elastic_fe() {
            Ok((base, changed)) => {
                s.push_str(&format!("1D elastic FE peak turn load share: baseline {:.2}%, changed {:.2}%\n",
                    100.0 * base.peak_fraction, 100.0 * changed.peak_fraction));
                s.push_str(&format!("FE parent annulus: uniform radial wall {:.4} in{}; no capacity correction\n",
                    self.file.parent_wall.unwrap_or(0.5 * self.file.case.nominal_major),
                    if self.file.parent_wall.is_none() { " (assumed 0.5D)" } else { " (entered minimum modeled uniformly)" }));
                s.push_str("FE load sharing excludes flank clearance, plasticity and rupture; coil and Keensert are not modeled.\n");
            }
            Err(reason) => s.push_str(&format!("Elastic FE load sharing unavailable: {reason}\n")),
        }
        s.push_str("Default strengths and load are illustrative. No safety factor, preload, fatigue, temperature or transverse load. Basic dimensions do not certify a thread class.\n");
        s
    }
}
impl Authoring for ThreadSession {
    fn initial_zoom(&self) -> Option<f32> { Some(1.25) }
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn key_labels(&self) -> [&'static str; 4] {
        ["Parent material", "Insert", "Bolt", "Section"]
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn act(&mut self, a: EditorAction) {
        if let EditorAction::Thread(a) = a {
            self.run(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        if let Some(p) = &mut self.prompt {
            match key {
                Key::Escape => self.prompt = None,
                Key::Enter => self.commit(),
                Key::Backspace => {
                    if p.fresh {
                        p.text.clear();
                        p.fresh = false;
                    } else {
                        p.text.pop();
                    }
                }
                Key::Char(c) => {
                    if p.text.len() < 200
                        && (p.field == Field::RatingSource
                            || c.is_ascii_digit()
                            || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
                    {
                        if p.fresh {
                            p.text.clear();
                            p.fresh = false;
                        }
                        p.text.push(c);
                    }
                }
                _ => {}
            }
            return KeyResult::Handled;
        }
        match key {
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'s') => self.save(),
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'z') => {
                if mods.shift {
                    self.redo()
                } else {
                    self.undo()
                }
            }
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'y') => self.redo(),
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    fn click(&mut self, _: [f32; 2], _: [f32; 2], _: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _: [f32; 2], _: [f32; 2], _: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        let shown = if self.changed_view {
            self.file.changed()
        } else {
            self.file.case.clone()
        };
        let (scene, translucent_from) = drawing::scene(&self.file.case, &shown, &self.file.name)?;
        self.translucent_from = translucent_from;
        self.drawn = true;
        Ok(scene)
    }
    fn needs_scene(&self) -> bool {
        !self.drawn
    }
    fn translucent(&self) -> Option<(u32, f32)> {
        self.translucent_from.map(|first| (first, 0.28))
    }
    fn initial_view(&self) -> Option<crate::gpu_viewer::controller::StandardView> {
        Some(crate::gpu_viewer::controller::StandardView::Isometric)
    }
    fn initial_style(&self) -> Option<crate::gpu_viewer::display::DisplayStyle> {
        Some(crate::gpu_viewer::display::DisplayStyle::Shaded)
    }
    fn initial_key_colours(&self) -> Option<[[u8; 3]; 4]> {
        Some([[108, 122, 131], [205, 214, 215], [168, 183, 192], [78, 88, 95]])
    }
    fn adjustable_shading(&self) -> bool { true }
    fn shows_body_only(&self) -> bool {
        true
    }
    fn poll(&mut self) -> bool {
        false
    }
    fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
    fn solving(&self) -> bool {
        false
    }
    fn status(&self) -> String {
        self.status.clone()
    }
    fn set_status(&mut self, s: String) {
        self.status = s
    }
    fn title(&self) -> String {
        format!(
            "Thread Workbench | {}{}",
            self.file.name,
            if self.dirty { " *" } else { "" }
        )
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn recovery_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&self.file).map_err(|e| e.to_string())
    }
    fn restore_recovery(&mut self) -> Result<(), String> {
        let s = std::fs::read_to_string(self.path.with_extension("recovery.json"))
            .map_err(|e| e.to_string())?;
        let file: File = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        file.validate()?;
        self.checkpoint();
        self.file = file;
        self.edited();
        Ok(())
    }
    fn tree(&self) -> Vec<crate::gpu_viewer::ui::TreeNode> {
        Vec::new()
    }
    fn properties(&self) -> Vec<DockItem> {
        super::authoring::flatten([self.inputs(), self.results()].concat())
    }
    fn inspector(&self) -> Vec<Card> {
        Vec::new()
    }
    fn ribbon(&self) -> Vec<crate::gpu_viewer::ui::RibbonTab> {
        Vec::new()
    }
    fn workbench(&self) -> Option<Workbench> {
        Some(self.workbench_data())
    }
    fn has_workbench(&self) -> bool {
        true
    }
    fn value_card(&self) -> Option<ValueCard> {
        let picker = self.picker.as_ref()?;
        let close = Self::action(ThreadAct::ClosePicker);
        match picker {
            Picker::Change => Some(ValueCard {
                title: "Damage characteristic".into(), width: 380.0, tag: "one change".into(),
                rows: vec![CardRow::Select {
                    label: "Characteristic".into(), selected: self.file.change.label().into(),
                    options: Change::ALL.into_iter().filter(|ch| ch.available(self.file.case.family))
                        .map(|ch| (ch.label().into(), Self::action(ThreadAct::Change(ch)))).collect(),
                }, CardRow::Text("Choose one measured change; edit its proposed value in Inputs.".into())],
                apply_label: "Close".into(), apply: close, cancel: close,
            }),
            Picker::Material { role, query } => {
                let current = match role {
                    MaterialRole::Bolt => &self.file.bolt_material,
                    MaterialRole::Parent => &self.file.parent_material,
                    MaterialRole::Insert => &self.file.insert_material,
                };
                let hits: Vec<_> = filter_sort(&Query { text: query.clone(), ..Default::default() })
                    .into_iter().filter(|&i| {
                        let m = catalog()[i];
                        m.fsu_ksi > 0.0 && (*role != MaterialRole::Bolt || m.ftu_ksi > 0.0)
                    }).collect();
                let mut shown: Vec<_> = hits.iter().copied().take(12).collect();
                if query.trim().is_empty() {
                    if let Some(i) = current.as_deref().and_then(|id| catalog().iter().position(|m| m.id == id)) {
                        if !shown.contains(&i) { shown.insert(0, i); shown.truncate(12); }
                    }
                }
                let mut rows = vec![CardRow::Field { label: "Search".into(), text: query.clone(), index: 0, unit: String::new() }];
                for i in shown {
                    let m = catalog()[i];
                    rows.push(CardRow::Choice {
                        label: m.name.into(), detail: format!("Ftu {:.1} · Fsu {:.1} ksi{}", m.ftu_ksi, m.fsu_ksi,
                            m.extra.map_or(" · typical".into(), |e| format!(" · {} · {}", e.form, e.basis))),
                        action: Self::action(ThreadAct::Material(*role, m.id)),
                        current: current.as_deref() == Some(m.id),
                    });
                }
                if hits.len() > 12 { rows.push(CardRow::Text(format!("{} more; type to narrow the list.", hits.len() - 12))); }
                if hits.is_empty() { rows.push(CardRow::Text("No matching material with the required shear strength.".into())); }
                rows.push(CardRow::Text("Catalog strengths are material data; verify form, condition and bolt product rating.".into()));
                Some(ValueCard {
                    title: match role { MaterialRole::Bolt => "Bolt material", MaterialRole::Parent => "Parent material", MaterialRole::Insert => "Insert material" }.into(),
                    width: 430.0, tag: "material library".into(), rows,
                    apply_label: "Close".into(), apply: close, cancel: close,
                })
            }
        }
    }
    fn card_field(&mut self, index: usize, text: String) {
        if index == 0 {
            if let Some(Picker::Material { query, .. }) = &mut self.picker { *query = text; }
        }
    }
    fn menus(&self) -> Vec<Menu> {
        vec![
            Menu {
                title: "File".into(),
                items: vec![
                    Item::Entry(Self::entry("Save case", ThreadAct::Save)),
                    Item::Entry(Self::entry("Report", ThreadAct::Report)),
                    Item::Entry(Self::entry("Evidence tables", ThreadAct::Evidence)),
                ],
            },
            Menu {
                title: "Edit".into(),
                items: vec![
                    Item::Entry(Self::entry("Undo", ThreadAct::Undo)),
                    Item::Entry(Self::entry("Redo", ThreadAct::Redo)),
                ],
            },
        ]
    }
    fn context_menu(&self) -> Vec<Item> {
        self.menus().into_iter().flat_map(|m| m.items).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tapped_hybrid_runs_and_keeps_analytical_capacity_separate() {
        let mut s = ThreadSession::open(&std::env::temp_dir().join("thread-hybrid-never-written.json")).unwrap();
        let (base, changed) = s.elastic_fe().unwrap();
        assert_eq!(base.per_turn_lbf.len(), 11);
        assert_eq!(base.per_turn_lbf, changed.per_turn_lbf);
        let fe_card = s.results().into_iter().find(|card| card.id == "thread-elastic-fe").unwrap();
        assert!(fe_card.items.iter().any(|item| matches!(item, DockItem::Plot(plot) if plot.series.len() == 2)));
        s.set_change(Change::FemaleMinor);
        let (base, changed) = s.elastic_fe().unwrap();
        assert_eq!(base.per_turn_lbf, changed.per_turn_lbf);
        let fe_card = s.results().into_iter().find(|card| card.id == "thread-elastic-fe").unwrap();
        assert!(fe_card.items.iter().any(|item| matches!(item, DockItem::Line { text, .. } if text.contains("curves overlap"))));
        let (analytical, damaged) = s.analyses();
        assert_ne!(analytical.rows, damaged.rows);
        s.set_change(Change::BoltMinor);
        assert_ne!(s.elastic_fe().unwrap().0.per_turn_lbf, s.elastic_fe().unwrap().1.per_turn_lbf);
        let capacity = s.analyses();
        let distribution = s.elastic_fe().unwrap().0.per_turn_lbf;
        s.file.parent_wall = Some(0.25);
        assert_eq!(s.analyses(), capacity);
        assert_ne!(s.elastic_fe().unwrap().0.per_turn_lbf, distribution);
        s.set_family(Family::HelicalCoil);
        assert!(s.elastic_fe().is_err());
    }
    #[test]
    fn damage_study_preserves_unselected_thread_properties() {
        let file = File::new();
        let base = file.case.clone();
        for (change, proposed) in [
            (Change::FemaleMinor, base.inner.female_minor + 0.002),
            (Change::BoltMinor, base.inner.minor - 0.002),
            (Change::BoltMajor, base.inner.major - 0.002),
            (Change::Tpi, 30.0),
            (Change::NominalMajor, 0.26),
        ] {
            let mut study = file.clone();
            study.change = change;
            study.proposed = proposed;
            let changed = study.changed();
            let mut expected = base.clone();
            match change {
                Change::FemaleMinor => expected.inner.female_minor = proposed,
                Change::BoltMinor => expected.inner.minor = proposed,
                Change::BoltMajor => expected.inner.major = proposed,
                Change::Tpi => expected.tpi = proposed,
                Change::NominalMajor => expected.nominal_major = proposed,
                _ => unreachable!(),
            }
            assert_eq!(changed, expected, "{change:?} changed another property");
            assert!(strength::compute(&changed).is_ok());
        }
    }
    #[test]
    fn evidence_tables_are_available_from_workbench_and_file_menu() {
        let s = ThreadSession::open(&std::env::temp_dir().join("thread-evidence-never-written.json")).unwrap();
        let wanted = ThreadSession::action(ThreadAct::Evidence);
        assert!(s.workbench_data().actions.iter().any(|entry| entry.action == wanted));
        assert!(s.menus().iter().find(|menu| menu.title == "File").unwrap().items.iter().any(|item| matches!(item, Item::Entry(entry) if entry.action == wanted)));
    }
    #[test]
    fn material_picker_searches_full_catalog_and_change_picker_lists_valid_damage() {
        let mut s = ThreadSession::open(&std::env::temp_dir().join("thread-picker-never-written.json")).unwrap();
        s.run(ThreadAct::OpenMaterial(MaterialRole::Bolt));
        s.card_field(0, "Inconel 718".into());
        let card = s.value_card().unwrap();
        assert!(card.rows.iter().any(|r| matches!(r, CardRow::Choice { label, .. } if label.contains("Inconel 718"))));
        s.run(ThreadAct::Material(MaterialRole::Bolt, "inconel718"));
        assert!(s.picker.is_none());
        assert_eq!(s.file.bolt_material.as_deref(), Some("inconel718"));
        s.run(ThreadAct::OpenChange);
        let card = s.value_card().unwrap();
        let CardRow::Select { options, .. } = &card.rows[0] else { panic!("damage dropdown"); };
        assert_eq!(options.len(), Change::ALL.iter().filter(|ch| ch.available(Family::Tapped)).count());
        assert!(options.iter().any(|(name, _)| name == Change::FemaleMinor.label()));
        s.run(ThreadAct::Change(Change::Engagement));
        assert!(s.picker.is_none());
        assert_eq!(s.file.change, Change::Engagement);
    }
    #[test]
    fn damage_comparison_and_rating_invalidation() {
        let path = std::env::temp_dir().join("thread-workbench-test-never-written.json");
        let mut s = ThreadSession::open(&path).unwrap();
        let (base, changed) = s.analyses();
        assert!(
            changed
                .rows
                .iter()
                .find(|r| r.name == "Bolt external thread stripping")
                .unwrap()
                .lbf
                < base
                    .rows
                    .iter()
                    .find(|r| r.name == "Bolt external thread stripping")
                    .unwrap()
                    .lbf
        );
        s.set_family(Family::HelicalCoil);
        assert!(s.analyses().1.governing.is_none());
        s.file.rating_value = Some(5000.0);
        s.file.rating_source = "Stanley chart".into();
        s.file.sync_rating();
        s.set_change(Change::BoltMajor);
        assert!(s.file.case.assembly_rating.is_some());
        assert!(s.analyses().1.governing.is_none());
    }
    #[test]
    fn saved_case_reopens_and_invalid_proposal_does_not_replace_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thread.json");
        let mut s = ThreadSession::open(&path).unwrap();
        s.run(ThreadAct::Save);
        assert_eq!(ThreadSession::open(&path).unwrap().file, s.file);
        let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        legacy.as_object_mut().unwrap().remove("parent_wall");
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(ThreadSession::open(&path).unwrap().file.parent_wall, None);
        s.edit(Field::Proposed);
        s.prompt.as_mut().unwrap().text = "0.500".into();
        s.commit();
        assert!(s.status.contains("unchanged"));
        assert!(s.file.proposed < s.file.case.inner.pitch);
    }
    #[test]
    fn small_thread_clearance_keeps_damage_selection_valid() {
        let mut s =
            ThreadSession::open(&std::env::temp_dir().join("thread-clearance-not-written.json"))
                .unwrap();
        s.file.case.inner.engagement = 0.001;
        s.set_change(Change::Engagement);
        assert!(s.analyses().1.governing.is_some());
        assert_eq!(s.file.proposed, 0.001);
    }
    #[test]
    fn ineffective_entry_turns_reduce_only_idealized_tapped_engagement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entry-loss.thread.json");
        let mut s = ThreadSession::open(&path).unwrap();
        s.set_change(Change::InactiveEntryTurns);
        s.file.proposed = 2.0;
        let (base, changed) = s.analyses();
        let remaining = 1.0 - 2.0 / (s.file.case.tpi * s.file.case.inner.engagement);
        for name in ["Bolt external thread stripping", "Parent internal thread stripping"] {
            let before = base.rows.iter().find(|r| r.name == name).unwrap().lbf;
            let after = changed.rows.iter().find(|r| r.name == name).unwrap().lbf;
            assert!((after / before - remaining).abs() < 1e-12);
        }
        assert_eq!(changed.rows[0].lbf, base.rows[0].lbf);
        s.file.proposed = 0.5;
        assert!(s.file.validate().is_err());
        s.file.proposed = 12.0;
        assert!(s.file.validate().is_err());
        s.file.proposed = 2.0;
        s.file.parent_wall = Some(0.125);
        s.run(ThreadAct::Save);
        assert_eq!(ThreadSession::open(&path).unwrap().file.parent_wall, Some(0.125));
        s.set_family(Family::HelicalCoil);
        assert_ne!(s.file.change, Change::InactiveEntryTurns);
    }
    #[test]
    fn proposed_value_uses_selected_characteristic_unit() {
        let mut s = ThreadSession::open(&std::env::temp_dir().join("thread-units-never-written.json")).unwrap();
        for (change, unit) in [(Change::Tpi, " TPI"), (Change::ParentShear, " ksi"), (Change::Load, " lbf"), (Change::InactiveEntryTurns, " turns")] {
            s.set_change(change);
            let DockItem::Field { value, .. } = s.row(Field::Proposed) else { panic!("proposed field"); };
            assert!(value.ends_with(unit), "{value}");
        }
        let DockItem::Field { value, .. } = s.row(Field::ParentWall) else { panic!("parent wall field"); };
        assert_eq!(value, "not supplied");
    }
    #[test]
    fn unchanged_insert_comparison_keeps_qualified_rating() {
        let mut s =
            ThreadSession::open(&std::env::temp_dir().join("thread-rating-not-written.json"))
                .unwrap();
        s.set_family(Family::HelicalCoil);
        s.file.rating_value = Some(5000.0);
        s.file.rating_source = "matched manufacturer chart".into();
        s.file.sync_rating();
        s.set_change(Change::Tpi);
        assert!(s.analyses().1.governing.is_some());
    }
    #[test]
    fn selected_library_material_sets_strengths_and_manual_edit_severs_link() {
        let mut s = ThreadSession::open(&std::env::temp_dir().join("thread-material-never-written.json")).unwrap();
        s.set_material(MaterialRole::Bolt, "ti6al4v");
        let m = File::material("ti6al4v").unwrap();
        assert_eq!(s.file.case.bolt_tensile_strength, m.ftu_ksi * 1000.0);
        assert_eq!(s.file.case.bolt_shear_strength, m.fsu_ksi * 1000.0);
        s.edit(Field::BoltShear);
        s.prompt.as_mut().unwrap().text = "80".into();
        s.commit();
        assert!(s.file.bolt_material.is_none());
        assert_eq!(s.file.case.bolt_shear_strength, 80_000.0);
        s.set_family(Family::Keensert);
        s.set_material(MaterialRole::Insert, "ph174");
        assert_eq!(s.file.case.insert_shear_strength, Some(File::material("ph174").unwrap().fsu_ksi * 1000.0));
        assert!(s.file.validate().is_ok());
    }
}
