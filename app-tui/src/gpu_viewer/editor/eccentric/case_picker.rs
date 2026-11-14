//! Choosing the Bushing case this window reads: the Bushing case files beside the Eccentric file, or any path typed in.
//! The choice is stored in the Eccentric file (`bushing_case`), relative when the case sits in the same folder.
use super::{EccentricAct, EccentricSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{bushing::file::BushingFile, session::EditorAction},
    ui::{CardRow, ValueCard},
};
use std::path::{Path, PathBuf};

/// Cases listed at once.
const SHOWN: usize = 12;
const TYPED: usize = 0;

pub struct CasePicker {
    typed: String,
    pub(super) files: Vec<PathBuf>,
}

fn act(a: EccentricAct) -> UiAction {
    UiAction::Editor(EditorAction::Eccentric(a))
}

/// A Bushing case file reads as one (`BushingFile` defaults every field, so any JSON object would parse): it carries the
/// `numbers` table the Bushing window writes and is not an Eccentric file.
fn is_bushing_case(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).ok().and_then(|v| v.as_object().map(|o| o.contains_key("numbers") && !o.contains_key("bushing_case"))).unwrap_or(false) && BushingFile::from_json(text).is_ok()
}

/// The Bushing case files in `dir`, by name.
fn scan(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")) && std::fs::read_to_string(p).is_ok_and(|t| is_bushing_case(&t)))
        .collect();
    found.sort();
    found
}

impl EccentricSession {
    pub(super) fn open_case_picker(&mut self) {
        let files = self.path.parent().map(scan).unwrap_or_default();
        self.case_picker = Some(CasePicker { typed: String::new(), files });
    }

    pub(super) fn case_picker_card(&self) -> Option<ValueCard> {
        let p = self.case_picker.as_ref()?;
        let current = self.case_path();
        let mut rows = vec![CardRow::Text(format!("Reading {}", current.display()))];
        for (i, f) in p.files.iter().enumerate().take(SHOWN) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            rows.push(CardRow::Choice { label: name, detail: super::case_age(f), action: act(EccentricAct::UseCase(i)), current: *f == current });
        }
        if p.files.is_empty() {
            rows.push(CardRow::Text("No Bushing case files beside this one: type a path.".into()));
        } else if p.files.len() > SHOWN {
            rows.push(CardRow::Text(format!("{} more beside this file: type a path.", p.files.len() - SHOWN)));
        }
        rows.push(CardRow::Field { label: "Other file".into(), text: p.typed.clone(), index: TYPED, unit: String::new() });
        rows.push(CardRow::Choice { label: "Use that file".into(), detail: String::new(), action: act(EccentricAct::UseTypedCase), current: false });
        Some(ValueCard { title: "Bushing case".into(), width: 440.0, tag: "from the Bushing workbench".into(), rows, apply_label: "Close".into(), apply: act(EccentricAct::ClosePicker), cancel: act(EccentricAct::ClosePicker) })
    }

    pub(super) fn case_picker_field(&mut self, index: usize, text: String) {
        if let (TYPED, Some(p)) = (index, &mut self.case_picker) {
            p.typed = text;
        }
    }

    /// Read from `path` from now on: refused (and nothing changes) unless it is a Bushing case.
    pub(super) fn use_case(&mut self, path: &Path) {
        match std::fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display())).and_then(|t| if is_bushing_case(&t) { Ok(()) } else { Err(format!("{} is not a Bushing case file.", path.display())) }) {
            Err(why) => self.status = why,
            Ok(()) => {
                let dir = self.path.parent().unwrap_or(Path::new(""));
                self.bushing_case = match path.strip_prefix(dir) {
                    Ok(rel) => rel.to_string_lossy().into_owned(),
                    Err(_) => path.to_string_lossy().into_owned(),
                };
                self.dirty = true;
                self.reload_bushing();
                self.case_picker = None;
            }
        }
    }

    pub(super) fn use_listed_case(&mut self, i: usize) {
        if let Some(path) = self.case_picker.as_ref().and_then(|p| p.files.get(i).cloned()) {
            self.use_case(&path);
        }
    }

    pub(super) fn use_typed_case(&mut self) {
        let Some(text) = self.case_picker.as_ref().map(|p| p.typed.trim().trim_matches('"').to_string()) else { return };
        if text.is_empty() {
            self.status = "Type the path of a Bushing case file.".into();
            return;
        }
        let typed = PathBuf::from(&text);
        let path = if typed.is_absolute() { typed } else { self.path.parent().unwrap_or(Path::new("")).join(typed) };
        self.use_case(&path);
    }
}
