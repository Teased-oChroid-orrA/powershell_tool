//! The Fastener Hole workbench in the native window: the regular-hole fit envelope and the countersink three-of-four solve
//! of `toolboxes::fastener_hole` (the same model, form rows and results), with the hole drawn to scale beside them.
//!
//! Inputs live in the inspector (a click types a number or cycles a choice), the results follow every edit in their own
//! cards, the tree lists both, the ribbon has the steps Inputs and Report. Nothing here computes a dimension: the
//! toolbox's `domain/` does, through `FastenerHoleModel::recompute`.
mod cards;
pub mod drawing;
pub mod file;

use super::{
    authoring::{Authoring, Card},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::gpu_viewer::{
    bench::Bench,
    controller::StandardView,
    menu::{Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
    Scene,
};
use crate::toolboxes::fastener_hole::{
    model::{self, FastenerHoleModel, FieldRow},
    view, FastenerHoleState,
};
use file::FastenerFile;
use std::path::{Path, PathBuf};

/// What a click in the Fastener Hole workbench asks for (carried by `EditorAction::Fastener`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FastenerAct {
    /// An input row: type the number, or cycle the choice.
    Row(FieldRow),
    /// Bring the tools of a step (the tree).
    Step(&'static str),
    Undo,
    Redo,
    Save,
    Export,
}

struct Prompt {
    row: FieldRow,
    text: String,
    /// Nothing typed yet: the first character replaces the value instead of extending it.
    fresh: bool,
}

pub struct FastenerSession {
    st: FastenerHoleState,
    path: PathBuf,
    name: String,
    dirty: bool,
    undo: Vec<FastenerFile>,
    redo: Vec<FastenerFile>,
    prompt: Option<Prompt>,
    status: String,
    step: &'static str,
    saved_revision: Option<String>,
    /// The picture is rebuilt only after an edit.
    drawn: bool,
}

const UNDO_LIMIT: usize = 100;

impl FastenerSession {
    pub fn new(file: &FastenerFile, path: PathBuf) -> Self {
        let mut st = FastenerHoleState::default();
        let skipped = file.apply(&mut st.model);
        st.clamp_selection();
        let status = if skipped.is_empty() { String::new() } else { format!("Not applied from the file: {}.", skipped.join("; ")) };
        Self {
            st,
            name: if file.name.is_empty() { path.file_stem().and_then(|n| n.to_str()).unwrap_or("Fastener hole").to_string() } else { file.name.clone() },
            saved_revision: crate::persistence::revision(&path),
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            status,
            step: "Inputs",
            drawn: false,
        }
    }

    /// A session on `path` when it holds a fastener hole case, else on the workbench defaults (saved there by Ctrl/Cmd+S).
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = if path.exists() { FastenerFile::from_json(&std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)? } else { FastenerFile::capture(&FastenerHoleModel::default(), "") };
        Ok(Self::new(&file, path.to_path_buf()))
    }

    pub fn model(&self) -> &FastenerHoleModel {
        &self.st.model
    }

    // ------------------------------------------------------------------ editing

    fn snapshot(&self) -> FastenerFile {
        FastenerFile::capture(&self.st.model, &self.name)
    }
    /// Record the state before an edit so Undo can return to it.
    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn changed(&mut self) {
        self.dirty = true;
        self.drawn = false;
        self.st.clamp_selection();
    }
    fn set_from(&mut self, file: &FastenerFile) {
        let skipped = file.apply(&mut self.st.model);
        if !skipped.is_empty() {
            self.status = format!("Not applied: {}.", skipped.join("; "));
        }
        self.prompt = None;
        self.changed();
    }
    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.set_from(&prev);
        }
    }
    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.set_from(&next);
        }
    }
    fn save(&mut self) {
        if crate::persistence::revision(&self.path) != self.saved_revision {
            self.status = "Save conflict: file changed externally; reopen before saving.".into();
            return;
        }
        match crate::persistence::write(&self.path, self.snapshot().to_json()) {
            Ok(()) => {
                self.dirty = false;
                self.saved_revision = crate::persistence::revision(&self.path);
                self.status = format!("Saved {}.", self.path.display());
            }
            Err(e) => self.status = format!("Could not save {}: {e}", self.path.display()),
        }
    }
    /// The terminal toolbox's report text, beside the case file (`name.report.txt`).
    fn export(&mut self) {
        let target = self.path.with_extension("report.txt");
        match crate::persistence::write(&target, view::build_report_text(&self.st.model)) {
            Ok(()) => self.status = format!("Report written to {}.", target.display()),
            Err(e) => self.status = format!("Could not write {}: {e}", target.display()),
        }
    }

    // ------------------------------------------------------------------ rows and prompts

    fn rows(&self) -> Vec<FieldRow> {
        model::field_rows(&self.st.model)
    }
    /// A click (or Enter) on an input row.
    fn activate_row(&mut self, row: FieldRow) {
        match row {
            FieldRow::Header(_) => {}
            FieldRow::Number(target, part, _) => {
                let text = model::format_for_edit(model::part_value(&self.st.model.get_toleranced(target), part));
                self.prompt = Some(Prompt { row, text, fresh: true });
            }
            _ => {
                let Some(i) = self.rows().iter().position(|r| *r == row) else { return };
                self.prompt = None;
                self.checkpoint();
                self.st.selected = i;
                self.st.toggle_selected();
                self.changed();
            }
        }
    }
    fn commit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        let FieldRow::Number(target, part, _) = p.row else { return };
        let label = view::row_label(p.row);
        let before = self.st.model.get_toleranced(target);
        match p.text.trim().parse::<f64>() {
            // Typing the value it already has is not an edit: no undo step.
            Ok(v) if v.is_finite() && v == model::part_value(&before, part) => {}
            Ok(v) if v.is_finite() => {
                self.checkpoint();
                self.st.model.commit_number(target, part, v);
                if self.st.model.get_toleranced(target) == before {
                    self.undo.pop();
                    self.status = format!("{label} not changed: {v} does not make a valid tolerance band.");
                } else {
                    self.changed();
                }
            }
            _ => self.status = format!("{label} not changed: {:?} is not a number.", p.text.trim()),
        }
    }
    fn prompt_char(&mut self, c: char) {
        let Some(p) = &mut self.prompt else { return };
        if (c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')) && p.text.len() < 40 {
            if std::mem::take(&mut p.fresh) {
                p.text.clear();
            }
            p.text.push(c);
        }
    }

    fn do_act(&mut self, a: FastenerAct) {
        // A message stays until the next thing the person does.
        self.status.clear();
        match a {
            FastenerAct::Row(r) => self.activate_row(r),
            FastenerAct::Step(s) => self.step = s,
            FastenerAct::Undo => self.undo(),
            FastenerAct::Redo => self.redo(),
            FastenerAct::Save => self.save(),
            FastenerAct::Export => self.export(),
        }
    }
}

impl Authoring for FastenerSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn key_labels(&self) -> [&'static str; 4] {
        drawing::key_labels(self.st.model.hole_type)
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::Fastener(a) = action {
            self.do_act(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        if self.prompt.is_some() {
            match key {
                Key::Escape => self.prompt = None,
                Key::Enter => self.commit_prompt(),
                Key::Backspace => {
                    if let Some(p) = &mut self.prompt {
                        if std::mem::take(&mut p.fresh) {
                            p.text.clear();
                        } else {
                            p.text.pop();
                        }
                    }
                }
                Key::Char(c) => self.prompt_char(c),
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
    // The picture is the inputs drawn; a click on it picks nothing.
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        self.drawn = true;
        // A case with nothing to draw shows an empty viewport; the drawing card says why.
        Ok(drawing::scene(&self.st.model, &self.name).map_or_else(|_| drawing::empty(&self.name), |(s, _)| s))
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
        if !self.status.is_empty() {
            return self.status.clone();
        }
        self.verdict()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn title(&self) -> String {
        format!("Fastener Hole | {}{}", self.name, if self.dirty { " *" } else { "" })
    }
    fn initial_view(&self) -> Option<StandardView> {
        Some(StandardView::Isometric)
    }
    fn restore_recovery(&mut self) -> Result<(), String> {
        let text = std::fs::read_to_string(self.path.with_extension("recovery.json")).map_err(|e| e.to_string())?;
        let f = FastenerFile::from_json(&text)?;
        self.undo.push(self.snapshot());
        self.set_from(&f);
        Ok(())
    }
    fn recovery_json(&self) -> Result<String, String> {
        Ok(self.snapshot().to_json())
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn focus_step(&self) -> Option<&'static str> {
        Some(self.step)
    }
    fn tree(&self) -> Vec<TreeNode> {
        self.tree_nodes()
    }
    fn properties(&self) -> Vec<DockItem> {
        super::authoring::flatten(self.inspector())
    }
    fn inspector(&self) -> Vec<Card> {
        self.inspector_cards()
    }
    fn ribbon(&self) -> Vec<RibbonTab> {
        self.ribbon_tabs()
    }
    fn step_state(&self, tab: &str) -> Option<super::authoring::StepState> {
        use super::authoring::StepState::{Done, Todo};
        Some(match tab {
            "Inputs" if self.valid() => Done,
            "Report" if !self.dirty && self.path.exists() => Done,
            "Inputs" | "Report" => Todo,
            _ => return None,
        })
    }
    fn menus(&self) -> Vec<Menu> {
        self.menu_bar()
    }
    fn context_menu(&self) -> Vec<Item> {
        self.menu_bar().into_iter().find(|m| m.title == "Tools").map(|m| m.items).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
