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
    ui::{Compare, CompareRow, DockItem, Look, Workbench, WorkbenchView},
    Scene,
};
use fastened_joint_solver::thread_capability::{
    self as strength, Analysis, Case, Family, Interface, StiMeasurement,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
}
impl Change {
    const ALL: [Self; 17] = [
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
        }
    }
    fn unit(self) -> &'static str {
        match self {
            Self::Tpi => "TPI",
            Self::ParentShear | Self::BoltTensile | Self::BoltShear => "ksi",
            Self::Load => "lbf",
            _ => "in",
        }
    }
    fn available(self, f: Family) -> bool {
        match self {
            Self::OuterMajor | Self::OuterFemaleMinor | Self::InsertLength => f == Family::Keensert,
            Self::StiMinor | Self::StiPitch => f == Family::HelicalCoil,
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
                c.inner = Interface::basic(v, c.tpi, c.inner.engagement);
            }
            Self::Tpi => {
                c.tpi = v;
                c.inner = Interface::basic(c.inner.major, v, c.inner.engagement);
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
}
impl Field {
    const ALL: [Self; 16] = [
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
}
impl File {
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
        let case = Case {
            family: Family::Tapped,
            nominal_major: 0.25,
            tpi: 28.0,
            inner: Interface::basic(0.25, 28.0, 0.375),
            outer: None,
            sti: None,
            axial_load: 3000.0,
            bolt_tensile_strength: 120_000.0,
            bolt_shear_strength: 72_000.0,
            parent_shear_strength: 30_000.0,
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
        strength::compute(&self.case).map_err(str::to_string)?;
        strength::compute(&self.changed()).map_err(|e| format!("proposed change: {e}"))?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadAct {
    Family(Family),
    Change(Change),
    NextChange,
    View(bool),
    Edit(usize),
    Undo,
    Redo,
    Save,
    Report,
}
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
    status: String,
    changed_view: bool,
    drawn: bool,
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
            status: String::new(),
            changed_view: false,
            drawn: false,
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
            }
            Family::HelicalCoil => {
                self.file.case.outer = None;
                self.file.case.insert_shear_strength = None;
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
        self.edited();
    }
    fn set_change(&mut self, ch: Change) {
        if !ch.available(self.file.case.family) {
            return;
        }
        self.checkpoint();
        self.file.change = ch;
        self.file.proposed = File::suggested(ch, &self.file.case);
        self.edited();
    }
    fn next_change(&mut self) {
        let i = Change::ALL
            .iter()
            .position(|x| *x == self.file.change)
            .unwrap_or(0);
        let ch = Change::ALL
            .iter()
            .cycle()
            .skip(i + 1)
            .find(|x| x.available(self.file.case.family))
            .copied()
            .unwrap();
        self.set_change(ch);
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
        }
    }
    fn edit(&mut self, f: Field) {
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
                Field::BoltTensile => next.case.bolt_tensile_strength = v * 1000.0,
                Field::BoltShear => next.case.bolt_shear_strength = v * 1000.0,
                Field::ParentShear => next.case.parent_shear_strength = v * 1000.0,
                Field::InsertShear => next.case.insert_shear_strength = Some(v * 1000.0),
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
            ThreadAct::Change(c) => self.set_change(c),
            ThreadAct::NextChange => self.next_change(),
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
        }
    }
    fn analyses(&self) -> (Analysis, Analysis) {
        (
            strength::compute(&self.file.case).expect("validated baseline"),
            strength::compute(&self.file.changed()).expect("validated proposal"),
        )
    }
    fn row(&self, f: Field) -> DockItem {
        let editing = self.prompt.as_ref().filter(|p| p.field == f);
        let value = editing.map_or_else(|| self.value(f), |p| p.text.clone());
        DockItem::field(
            f.label(),
            format!(
                "{}{}{}",
                value,
                if editing.is_some() { "_" } else { "" },
                if editing.is_some() { "" } else { f.unit() }
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
        let load=Card::new("thread-load","Load and strengths",Relevance::Primary,vec![
            self.row(Field::Load),self.row(Field::BoltTensile),self.row(Field::BoltShear),
            self.row(Field::ParentShear),
            DockItem::text("Default strengths and load are illustrative. Enter documented strengths for your actual materials.",Tone::Warn),
            DockItem::text("Results are an axial screen, not design allowables.",Tone::Dim)]);
        let mut cards = vec![family, geometry, load];
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
                rows.extend([
                    self.row(Field::InsertShear),
                    self.row(Field::OuterMajor),
                    self.row(Field::OuterTpi),
                    self.row(Field::OuterLength),
                ]);
            }
            if c.family == Family::HelicalCoil {
                if let Some(l) = strength::sti_limits(c.nominal_major, c.tpi) {
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
            DockItem::Segmented([(Change::FemaleMinor,"Internal minor ↑"),(Change::BoltMajor,"Bolt major ↓"),
                (Change::BoltMinor,"Bolt root ↓"),(Change::Engagement,"Engagement loss")]
                .into_iter().map(|(ch,l)|Self::entry(l,ThreadAct::Change(ch)).checked(self.file.change==ch)).collect()),
            DockItem::field("Other characteristic",self.file.change.label(),Tone::Normal,
                Some(Self::action(ThreadAct::NextChange)),false),
            DockItem::field("Baseline value",format!("{:.4} {}",self.file.change.current(c),self.file.change.unit()),Tone::Dim,None,false),
            self.row(Field::Proposed),
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
        vec![
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
        ]
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
            model_hint: "Schematic section only. Strength values are in the comparison table."
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
        s.push_str("Default strengths and load are illustrative. No safety factor, preload, fatigue, temperature or transverse load. Basic dimensions do not certify a thread class.\n");
        s
    }
    fn schematic(&self) -> Scene {
        let c = if self.changed_view {
            self.file.changed()
        } else {
            self.file.case.clone()
        };
        let insert = c.family != Family::Tapped;
        let bolt_half = (c.inner.major / self.file.case.inner.major * 0.16).clamp(0.05, 0.3) as f32;
        let gap = (c.inner.female_minor / self.file.case.inner.female_minor * 0.19)
            .clamp(0.16, 0.35) as f32;
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        let mut samples = Vec::new();
        let mut rect = |x0: f32, x1: f32, y0: f32, y1: f32, code: f32| {
            let n = positions.len() as u32;
            positions.extend([[x0, y0, 0.0], [x1, y0, 0.0], [x1, y1, 0.0], [x0, y1, 0.0]]);
            triangles.extend([n, n + 1, n + 2, n, n + 2, n + 3]);
            samples.extend([[0.0, 0.0, 0.0, code]; 4]);
        };
        // The native viewer's surface-less style hides state codes below 0.5.
        rect(-0.9, -gap, -0.6, 0.6, 0.5);
        rect(gap, 0.9, -0.6, 0.6, 0.5);
        if insert {
            rect(-gap, -bolt_half - 0.015, -0.45, 0.45, 1.0);
            rect(bolt_half + 0.015, gap, -0.45, 0.45, 1.0);
        }
        rect(-bolt_half, bolt_half, -0.78, 0.78, 2.0);
        let crest = bolt_half + if insert { 0.012 } else { 0.025 };
        for tooth in 0..8 {
            let y0 = -0.5 + tooth as f32 * 0.125;
            let y1 = y0 + 0.125;
            for side in [-1.0_f32, 1.0] {
                let n = positions.len() as u32;
                positions.extend([
                    [side * bolt_half, y0, 0.001],
                    [side * crest, (y0 + y1) / 2.0, 0.001],
                    [side * bolt_half, y1, 0.001],
                ]);
                if side < 0.0 {
                    triangles.extend([n, n + 2, n + 1]);
                } else {
                    triangles.extend([n, n + 1, n + 2]);
                }
                samples.extend([[0.0, 0.0, 0.0, 2.0]; 3]);
            }
        }
        Scene {
            label: format!("{} | schematic thread section", self.file.name),
            positions,
            triangles,
            samples,
            times: vec![0.0],
            range: [0.0, 3.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
        }
    }
}
impl Authoring for ThreadSession {
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
        self.drawn = true;
        Ok(self.schematic())
    }
    fn needs_scene(&self) -> bool {
        !self.drawn
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
    fn menus(&self) -> Vec<Menu> {
        vec![
            Menu {
                title: "File".into(),
                items: vec![
                    Item::Entry(Self::entry("Save case", ThreadAct::Save)),
                    Item::Entry(Self::entry("Report", ThreadAct::Report)),
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
}
