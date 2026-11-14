//! Library management card: add, edit, delete, tag Preferred, import and export for the three saved libraries behind the
//! Bushing pickers (reamer sizes, materials, bushing IDs). The file format, the duplicate pruning and the conflict rules
//! are `crate::library`'s; the files are the toolbox's `*_persistence` modules. One generic [`Manager`] over a small
//! [`Catalog`] per library, driven by the window through the object-safe [`Manage`].
use super::BushingAct;
use crate::gpu_viewer::{
    controller::UiAction,
    editor::session::EditorAction,
    ui::{CardRow, ValueCard},
};
use crate::library::{self, ConflictQueue, ConflictResolution, LibraryItem};
use crate::toolboxes::bushing::{
    bushing_id_persistence::{self as ids, PersistedBushingId},
    material_persistence::{self as mats, PersistedMaterial},
    model::{format_for_edit, BushingModel},
    reamer_persistence::{self as reamers, PersistedReamer},
};
use serde::{de::DeserializeOwned, Serialize};
use std::path::{Path, PathBuf};

/// The label that marks an entry a picker lists first.
pub const PREFERRED: &str = "Preferred";
/// Entries listed at once; the search narrows the rest.
const SHOWN: usize = 10;
const SEARCH: usize = 0;
const IMPORT_PATH: usize = 99;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ManageAct {
    New,
    /// The `i`th entry of the list for the current search.
    Edit(usize),
    Save,
    Delete,
    Preferred,
    Import,
    Export,
    /// Resolve the import conflict on screen: overwrite (else keep), and every remaining one the same way.
    Resolve { overwrite: bool, all: bool },
    /// Leave the form or conflict view for the list; from the list, close.
    Back,
}

fn act(a: ManageAct) -> UiAction {
    UiAction::Editor(EditorAction::Bushing(BushingAct::Manage(a)))
}

/// What one library is: its fields, how an entry is keyed, read from the form and written back.
pub trait Catalog {
    type T: Clone + PartialEq + Serialize + DeserializeOwned;
    const TITLE: &'static str;
    /// Form labels, in the order `to_fields` / `from_fields` use.
    const FIELDS: &'static [&'static str];
    /// What makes two entries "the same one" (the library's import key).
    fn key(t: &Self::T) -> String;
    fn summary(t: &Self::T) -> String;
    fn to_fields(t: &Self::T) -> Vec<String>;
    fn from_fields(f: &[String]) -> Result<Self::T, String>;
    fn load_from(path: &Path) -> Option<Vec<LibraryItem<Self::T>>>;
    fn save_to(path: &Path, items: &[LibraryItem<Self::T>]);
    /// Entries that ship with the app: listed read-only (their Preferred tag can still be set) and written by Export.
    fn builtin() -> Vec<LibraryItem<Self::T>> {
        Vec::new()
    }
    /// A key the app already uses outside this library.
    fn reserved(_key: &str) -> bool {
        false
    }
    /// Hand the model the edited library.
    fn refresh(items: &[LibraryItem<Self::T>], m: &mut BushingModel);
    /// Stop a delete the case still depends on; fix the model's references after one.
    fn guard_delete(_pos: usize, _m: &BushingModel) -> Result<(), String> {
        Ok(())
    }
    fn after_delete(_pos: usize, _m: &mut BushingModel) {}
}

fn num(f: &[String], i: usize, name: &str, ok: impl Fn(f64) -> bool, rule: &str) -> Result<f64, String> {
    let v: f64 = f[i].trim().parse().map_err(|_| format!("{name}: {:?} is not a number", f[i].trim()))?;
    if v.is_finite() && ok(v) {
        Ok(v)
    } else {
        Err(format!("{name} must be {rule}"))
    }
}
fn name(f: &[String], i: usize, what: &str) -> Result<String, String> {
    let s = f[i].trim();
    if s.is_empty() {
        Err(format!("{what} is empty"))
    } else {
        Ok(s.to_string())
    }
}

pub struct ReamerCat;
impl Catalog for ReamerCat {
    type T = PersistedReamer;
    const TITLE: &'static str = "Reamer library";
    const FIELDS: &'static [&'static str] = &["Size label", "Nominal (in)", "Tool tolerance + (in)", "Tool tolerance - (in)", "Notes"];
    fn key(t: &PersistedReamer) -> String {
        t.size_label.clone()
    }
    fn summary(t: &PersistedReamer) -> String {
        format!("{:.4} in  +{:.4}/-{:.4}", t.nominal_in, t.tool_tolerance_plus_in, t.tool_tolerance_minus_in)
    }
    fn to_fields(t: &PersistedReamer) -> Vec<String> {
        vec![t.size_label.clone(), format_for_edit(t.nominal_in), format_for_edit(t.tool_tolerance_plus_in), format_for_edit(t.tool_tolerance_minus_in), t.notes.clone()]
    }
    fn from_fields(f: &[String]) -> Result<PersistedReamer, String> {
        Ok(PersistedReamer {
            size_label: name(f, 0, "Size label")?,
            nominal_in: num(f, 1, "Nominal", |v| v > 0.0, "a positive diameter")?,
            tool_tolerance_plus_in: num(f, 2, "Tool tolerance +", |v| v >= 0.0, "zero or more")?,
            tool_tolerance_minus_in: num(f, 3, "Tool tolerance -", |v| v >= 0.0, "zero or more")?,
            notes: f[4].trim().to_string(),
        })
    }
    fn load_from(path: &Path) -> Option<Vec<LibraryItem<PersistedReamer>>> {
        reamers::load_from(path)
    }
    fn save_to(path: &Path, items: &[LibraryItem<PersistedReamer>]) {
        reamers::save_to(path, items)
    }
    fn builtin() -> Vec<LibraryItem<PersistedReamer>> {
        reamers::builtin_as_library_items()
    }
    fn refresh(items: &[LibraryItem<PersistedReamer>], m: &mut BushingModel) {
        m.set_user_reamers(items);
    }
}

pub struct MaterialCat;
impl Catalog for MaterialCat {
    type T = PersistedMaterial;
    const TITLE: &'static str = "Material library";
    const FIELDS: &'static [&'static str] = &["Name", "E (ksi)", "Fty (ksi)", "Fbru, e/D 2 (ksi)", "Fbru, e/D 1.5 (ksi, 0 = none)", "Fsu (ksi)", "Ftu (ksi)", "Poisson ratio", "CTE (uin/in/F)"];
    fn key(t: &PersistedMaterial) -> String {
        t.name.clone()
    }
    fn summary(t: &PersistedMaterial) -> String {
        format!("E {:.0} ksi, Fty {:.0} ksi, Ftu {:.0} ksi", t.e_ksi, t.sy_ksi, t.ftu_ksi)
    }
    fn to_fields(t: &PersistedMaterial) -> Vec<String> {
        vec![t.name.clone(), format_for_edit(t.e_ksi), format_for_edit(t.sy_ksi), format_for_edit(t.fbru_ksi), format_for_edit(t.fbru_e15_ksi), format_for_edit(t.fsu_ksi), format_for_edit(t.ftu_ksi), format_for_edit(t.nu), format_for_edit(t.alpha_u_f)]
    }
    fn from_fields(f: &[String]) -> Result<PersistedMaterial, String> {
        let pos = |v: f64| v > 0.0;
        Ok(PersistedMaterial {
            name: name(f, 0, "Name")?,
            e_ksi: num(f, 1, "E", pos, "positive")?,
            sy_ksi: num(f, 2, "Fty", pos, "positive")?,
            fbru_ksi: num(f, 3, "Fbru", pos, "positive")?,
            fbru_e15_ksi: num(f, 4, "Fbru at e/D 1.5", |v| v >= 0.0, "zero or more")?,
            fsu_ksi: num(f, 5, "Fsu", pos, "positive")?,
            ftu_ksi: num(f, 6, "Ftu", pos, "positive")?,
            nu: num(f, 7, "Poisson ratio", |v| v > 0.0 && v < 0.5, "between 0 and 0.5")?,
            alpha_u_f: num(f, 8, "CTE", |_| true, "a number")?,
        })
    }
    fn load_from(path: &Path) -> Option<Vec<LibraryItem<PersistedMaterial>>> {
        mats::load_from(path)
    }
    fn save_to(path: &Path, items: &[LibraryItem<PersistedMaterial>]) {
        mats::save_to(path, items)
    }
    fn reserved(key: &str) -> bool {
        mechanics_core::materials::builtin_catalog().any(|m| m.name.eq_ignore_ascii_case(key))
    }
    fn refresh(items: &[LibraryItem<PersistedMaterial>], m: &mut BushingModel) {
        m.set_custom_materials(items);
    }
    fn guard_delete(pos: usize, m: &BushingModel) -> Result<(), String> {
        let at = mechanics_core::materials::builtin_len() + pos;
        if m.housing_material_index == at || m.bushing_material_index == at {
            Err("This material is in use by the case: pick another material first.".into())
        } else {
            Ok(())
        }
    }
    fn after_delete(pos: usize, m: &mut BushingModel) {
        let at = mechanics_core::materials::builtin_len() + pos;
        for i in [&mut m.housing_material_index, &mut m.bushing_material_index] {
            if *i > at {
                *i -= 1;
            }
        }
    }
}

pub struct IdCat;
impl Catalog for IdCat {
    type T = PersistedBushingId;
    const TITLE: &'static str = "Bushing ID library";
    const FIELDS: &'static [&'static str] = &["Label", "ID (in)"];
    fn key(t: &PersistedBushingId) -> String {
        t.label.clone()
    }
    fn summary(t: &PersistedBushingId) -> String {
        format!("{:.4} in", t.id_in)
    }
    fn to_fields(t: &PersistedBushingId) -> Vec<String> {
        vec![t.label.clone(), format_for_edit(t.id_in)]
    }
    fn from_fields(f: &[String]) -> Result<PersistedBushingId, String> {
        Ok(PersistedBushingId { label: name(f, 0, "Label")?, id_in: num(f, 1, "ID", |v| v > 0.0, "a positive diameter")? })
    }
    fn load_from(path: &Path) -> Option<Vec<LibraryItem<PersistedBushingId>>> {
        ids::load_from(path)
    }
    fn save_to(path: &Path, items: &[LibraryItem<PersistedBushingId>]) {
        ids::save_to(path, items)
    }
    fn refresh(_: &[LibraryItem<PersistedBushingId>], _: &mut BushingModel) {}
}

/// The library file a catalog lives in, under `dir` or the app's data folder.
pub fn library_file(kind: Lib, dir: Option<&Path>) -> PathBuf {
    let default = match kind {
        Lib::Reamer => reamers::default_file_path(),
        Lib::Material => mats::default_file_path(),
        Lib::Id => ids::default_file_path(),
    };
    match (dir, default.file_name()) {
        (Some(d), Some(f)) => d.join(f),
        _ => default,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lib {
    Reamer,
    Material,
    Id,
}

/// One row of the list: a shipped entry or one of the user's, with its tags.
struct Entry<T> {
    item: T,
    labels: Vec<String>,
    builtin: bool,
    /// Position in the user's file (none for a shipped entry nobody tagged).
    user: Option<usize>,
}

struct Form<T> {
    editing: Option<Entry<T>>,
    texts: Vec<String>,
    preferred: bool,
}

enum View<T> {
    List,
    Form(Form<T>),
    Conflict(ConflictQueue<T>),
}

pub struct Manager<C: Catalog> {
    path: PathBuf,
    export_path: PathBuf,
    items: Vec<LibraryItem<C::T>>,
    query: String,
    import_path: String,
    note: String,
    view: View<C::T>,
    /// What the last import did, shown when its conflicts are settled.
    tally: (usize, usize),
}

/// What the window asks of the open library card; object safe so it holds one whatever the library.
pub trait Manage {
    fn card(&self) -> ValueCard;
    fn field(&mut self, index: usize, text: String);
    /// Run an action; false once the card is closed.
    fn act(&mut self, a: ManageAct, m: &mut BushingModel) -> bool;
}

pub fn open(kind: Lib, dir: Option<&Path>, export_dir: &Path) -> Box<dyn Manage> {
    fn make<C: Catalog + 'static>(path: PathBuf, export: PathBuf) -> Box<dyn Manage> {
        let items = C::load_from(&path).unwrap_or_default();
        Box::new(Manager::<C> { path, export_path: export, items, query: String::new(), import_path: String::new(), note: String::new(), view: View::List, tally: (0, 0) })
    }
    let path = library_file(kind, dir);
    let export = export_dir.join(path.file_stem().map(|s| format!("{}-export.json", s.to_string_lossy())).unwrap_or_else(|| "library-export.json".into()));
    match kind {
        Lib::Reamer => make::<ReamerCat>(path, export),
        Lib::Material => make::<MaterialCat>(path, export),
        Lib::Id => make::<IdCat>(path, export),
    }
}

impl<C: Catalog> Manager<C> {
    /// Shipped entries then the user's, filtered by the search.
    fn visible(&self) -> Vec<Entry<C::T>> {
        let q = self.query.trim().to_lowercase();
        let tags = |item: &C::T| self.items.iter().position(|u| &u.item == item);
        let mut all: Vec<Entry<C::T>> = C::builtin()
            .into_iter()
            .map(|b| {
                let user = tags(&b.item);
                Entry { labels: user.map_or(Vec::new(), |u| self.items[u].labels.clone()), item: b.item, builtin: true, user }
            })
            .collect();
        for (i, u) in self.items.iter().enumerate() {
            if !all.iter().any(|e| e.user == Some(i)) {
                all.push(Entry { item: u.item.clone(), labels: u.labels.clone(), builtin: false, user: Some(i) });
            }
        }
        all.retain(|e| q.is_empty() || C::key(&e.item).to_lowercase().contains(&q) || e.labels.iter().any(|l| l.to_lowercase().contains(&q)));
        all
    }

    fn key_taken(&self, key: &str, except_user: Option<usize>) -> bool {
        let same = |k: &str| k.eq_ignore_ascii_case(key);
        C::reserved(key) || C::builtin().iter().any(|b| same(&C::key(&b.item))) || self.items.iter().enumerate().any(|(i, u)| Some(i) != except_user && same(&C::key(&u.item)))
    }

    /// Write the file, then read it back: the library is only changed if the disk has it.
    fn persist(&mut self, m: &mut BushingModel) -> bool {
        C::save_to(&self.path, &self.items);
        if C::load_from(&self.path).as_ref() != Some(&self.items) {
            self.note = format!("Could not save {}.", self.path.display());
            return false;
        }
        C::refresh(&self.items, m);
        true
    }

    fn open_form(&mut self, editing: Option<Entry<C::T>>) {
        let texts = editing.as_ref().map_or_else(|| vec![String::new(); C::FIELDS.len()], |e| C::to_fields(&e.item));
        let preferred = editing.as_ref().is_some_and(|e| e.labels.iter().any(|l| l == PREFERRED));
        self.view = View::Form(Form { editing, texts, preferred });
        self.note.clear();
    }

    /// Set or clear Preferred on a library entry, dropping a tag-only copy of a shipped entry once it carries no tag.
    fn set_preferred(&mut self, e: &Entry<C::T>, on: bool) {
        let has = |labels: &[String]| labels.iter().any(|l| l == PREFERRED);
        match e.user {
            Some(u) => {
                self.items[u].labels.retain(|l| l != PREFERRED);
                if on {
                    self.items[u].labels.push(PREFERRED.into());
                }
                if e.builtin && self.items[u].labels.is_empty() {
                    self.items.remove(u);
                }
            }
            None if on && !has(&e.labels) => self.items.push(LibraryItem { item: e.item.clone(), labels: vec![PREFERRED.into()] }),
            None => {}
        }
    }

    fn save_form(&mut self, m: &mut BushingModel) {
        let View::Form(form) = &self.view else { return };
        let item = match C::from_fields(&form.texts) {
            Ok(i) => i,
            Err(e) => {
                self.note = e;
                return;
            }
        };
        let editing = form.editing.as_ref();
        if editing.is_some_and(|e| e.builtin) {
            self.note = "A shipped entry cannot be changed: add your own copy under another name.".into();
            return;
        }
        let key = C::key(&item);
        if self.key_taken(&key, editing.and_then(|e| e.user)) {
            self.note = format!("{key:?} is already in the library.");
            return;
        }
        let mut labels = editing.map(|e| e.labels.clone()).unwrap_or_default();
        labels.retain(|l| l != PREFERRED);
        if form.preferred {
            labels.push(PREFERRED.into());
        }
        let entry = LibraryItem { item, labels };
        match editing.and_then(|e| e.user) {
            Some(u) => self.items[u] = entry,
            None => self.items.push(entry),
        }
        if self.persist(m) {
            self.note = format!("Saved {key}.");
            self.view = View::List;
        }
    }

    fn delete(&mut self, m: &mut BushingModel) {
        let View::Form(Form { editing: Some(e), .. }) = &self.view else { return };
        let (Some(u), false) = (e.user, e.builtin) else {
            self.note = "A shipped entry cannot be deleted.".into();
            return;
        };
        if let Err(why) = C::guard_delete(u, m) {
            self.note = why;
            return;
        }
        let key = C::key(&e.item);
        let removed = self.items.remove(u);
        if self.persist(m) {
            C::after_delete(u, m);
            self.note = format!("Deleted {key}.");
            self.view = View::List;
        } else {
            self.items.insert(u, removed);
        }
    }

    fn export(&mut self) {
        let mut all = C::builtin();
        for u in &self.items {
            match all.iter_mut().find(|b| b.item == u.item) {
                Some(b) => {
                    let fresh: Vec<String> = u.labels.iter().filter(|l| !b.labels.contains(l)).cloned().collect();
                    b.labels.extend(fresh);
                }
                None => all.push(u.clone()),
            }
        }
        self.note = match std::fs::write(&self.export_path, library::export_json(&all)) {
            Ok(()) => format!("Exported {} entries to {}.", all.len(), self.export_path.display()),
            Err(e) => format!("Could not write {}: {e}", self.export_path.display()),
        };
    }

    fn import(&mut self, m: &mut BushingModel) {
        let path = self.import_path.trim().trim_matches('"');
        if path.is_empty() {
            self.note = "Type the path of a library file to import.".into();
            return;
        }
        let incoming = match std::fs::read_to_string(path).map_err(|e| format!("Could not read {path}: {e}")).and_then(|t| library::import_json::<C::T>(&t)) {
            Ok(i) => i,
            Err(e) => {
                self.note = e;
                return;
            }
        };
        // A shipped entry with no tag adds nothing.
        let shipped = C::builtin();
        let incoming: Vec<_> = incoming.into_iter().filter(|i| !(i.labels.is_empty() && shipped.iter().any(|b| b.item == i.item))).collect();
        let (queue, added, merged) = ConflictQueue::new(library::classify_import(&self.items, incoming, C::key), &mut self.items);
        self.tally = (added, merged);
        if queue.is_empty() {
            self.finish_import(m, 0);
        } else {
            self.view = View::Conflict(queue);
        }
    }

    fn finish_import(&mut self, m: &mut BushingModel, resolved: usize) {
        self.view = View::List;
        if self.persist(m) {
            self.note = format!("Imported: {} added, {} tag merges, {resolved} conflicts settled.", self.tally.0, self.tally.1);
        }
    }

    fn resolve(&mut self, m: &mut BushingModel, overwrite: bool, all: bool) {
        let View::Conflict(q) = &mut self.view else { return };
        let before = q.remaining();
        let resolution = if overwrite { ConflictResolution::Overwrite } else { ConflictResolution::KeepExisting };
        q.resolve(&mut self.items, resolution, all);
        if q.is_empty() {
            self.finish_import(m, before);
        }
    }

    fn list_rows(&self) -> Vec<CardRow> {
        let mut rows = vec![CardRow::Field { label: "Search".into(), text: self.query.clone(), index: SEARCH, unit: String::new() }];
        let vis = self.visible();
        for (i, e) in vis.iter().enumerate().take(SHOWN) {
            let tags = if e.labels.is_empty() { String::new() } else { format!("  {}", e.labels.join(", ")) };
            let src = if e.builtin { "shipped  " } else { "" };
            rows.push(CardRow::Choice { label: C::key(&e.item), detail: format!("{src}{}{tags}", C::summary(&e.item)), action: act(ManageAct::Edit(i)), current: e.labels.iter().any(|l| l == PREFERRED) });
        }
        if vis.len() > SHOWN {
            rows.push(CardRow::Text(format!("{} more: type to narrow the list.", vis.len() - SHOWN)));
        }
        rows.push(CardRow::Choice { label: "Add an entry".into(), detail: String::new(), action: act(ManageAct::New), current: false });
        rows.push(CardRow::Field { label: "Import file".into(), text: self.import_path.clone(), index: IMPORT_PATH, unit: String::new() });
        rows.push(CardRow::Choice { label: "Import".into(), detail: "adds new entries; asks on a conflict".into(), action: act(ManageAct::Import), current: false });
        rows.push(CardRow::Choice { label: "Export".into(), detail: self.export_path.display().to_string(), action: act(ManageAct::Export), current: false });
        rows
    }
}

impl<C: Catalog> Manage for Manager<C> {
    fn card(&self) -> ValueCard {
        let mut rows = Vec::new();
        if !self.note.is_empty() {
            rows.push(CardRow::Text(self.note.clone()));
        }
        let (title, tag, apply_label) = match &self.view {
            View::List => {
                rows.extend(self.list_rows());
                (C::TITLE.to_string(), "saved", "Close")
            }
            View::Form(f) => {
                let builtin = f.editing.as_ref().is_some_and(|e| e.builtin);
                for (i, label) in C::FIELDS.iter().enumerate() {
                    if builtin {
                        rows.push(CardRow::Text(format!("{label}: {}", f.texts[i])));
                    } else {
                        rows.push(CardRow::Field { label: (*label).into(), text: f.texts[i].clone(), index: i + 1, unit: String::new() });
                    }
                }
                rows.push(CardRow::Toggle { label: PREFERRED.into(), on: f.preferred, action: act(ManageAct::Preferred) });
                if f.editing.as_ref().is_some_and(|e| !e.builtin) {
                    rows.push(CardRow::Choice { label: "Delete this entry".into(), detail: String::new(), action: act(ManageAct::Delete), current: false });
                }
                (if f.editing.is_some() { "Edit entry".to_string() } else { "New entry".to_string() }, if builtin { "shipped" } else { "saved" }, if builtin { "Back" } else { "Save" })
            }
            View::Conflict(q) => {
                if let Some((at, incoming)) = q.current() {
                    rows.push(CardRow::Text(format!("{} is already saved with different data.", C::key(&incoming.item))));
                    if let Some(existing) = self.items.get(at) {
                        rows.push(CardRow::Text(format!("Saved: {}", C::summary(&existing.item))));
                    }
                    rows.push(CardRow::Text(format!("File:  {}", C::summary(&incoming.item))));
                }
                for (label, overwrite, all) in [("Keep saved", false, false), ("Overwrite", true, false), ("Keep saved, all remaining", false, true), ("Overwrite, all remaining", true, true)] {
                    rows.push(CardRow::Choice { label: label.into(), detail: String::new(), action: act(ManageAct::Resolve { overwrite, all }), current: false });
                }
                (format!("{}: import conflict", C::TITLE), "import", "Back")
            }
        };
        ValueCard { title, width: 420.0, tag: tag.into(), rows, apply_label: apply_label.into(), apply: act(if apply_label == "Save" { ManageAct::Save } else { ManageAct::Back }), cancel: act(ManageAct::Back) }
    }

    fn field(&mut self, index: usize, text: String) {
        match (&mut self.view, index) {
            (View::List, SEARCH) => self.query = text,
            (View::List, IMPORT_PATH) => self.import_path = text,
            (View::Form(f), i) if i >= 1 && i <= f.texts.len() => f.texts[i - 1] = text,
            _ => {}
        }
    }

    fn act(&mut self, a: ManageAct, m: &mut BushingModel) -> bool {
        match a {
            ManageAct::New => self.open_form(None),
            ManageAct::Edit(i) => {
                if let Some(e) = self.visible().into_iter().nth(i) {
                    self.open_form(Some(e));
                }
            }
            ManageAct::Save => self.save_form(m),
            ManageAct::Delete => self.delete(m),
            ManageAct::Preferred => {
                if let View::Form(f) = &mut self.view {
                    f.preferred = !f.preferred;
                    // A shipped entry has no other field to save: the tag is applied now.
                    if let Some(e) = f.editing.as_ref().filter(|e| e.builtin) {
                        let (e, on) = (Entry { item: e.item.clone(), labels: e.labels.clone(), builtin: true, user: e.user }, f.preferred);
                        self.set_preferred(&e, on);
                        if self.persist(m) {
                            self.note = format!("{} {}.", C::key(&e.item), if on { "is Preferred" } else { "is no longer Preferred" });
                            self.view = View::List;
                        }
                    }
                }
            }
            ManageAct::Import => self.import(m),
            ManageAct::Export => self.export(),
            ManageAct::Resolve { overwrite, all } => self.resolve(m, overwrite, all),
            ManageAct::Back => match self.view {
                View::List => return false,
                View::Form(_) => self.view = View::List,
                // Leaving an import half done keeps the saved entries and the ones already added.
                View::Conflict(_) => {
                    let (added, merged) = self.tally;
                    self.view = View::List;
                    if self.persist(m) {
                        self.note = format!("Import stopped: {added} added, {merged} tag merges; the other conflicts kept the saved entries.");
                    }
                }
            },
        }
        true
    }
}
