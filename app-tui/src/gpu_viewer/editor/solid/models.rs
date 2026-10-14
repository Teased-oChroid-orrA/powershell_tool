//! Whole-model commands of the solid editor: start a new empty part, load any template, open a problem or mesh
//! file, and clear what is loaded. Each replaces the model as one undoable edit; none of them keeps the previous
//! model's geometry, boundary conditions or feature history.
use super::*;
use crate::gpu_viewer::{
    controller::UiAction,
    editor::session::EditorAction,
    menu::{Entry, Item},
};

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(a))
}

impl SolidSession {
    /// Why the loaded model cannot be replaced now (`Ok` when it can).
    pub fn replace_availability(&self) -> Result<(), String> {
        if self.sketch_mode() {
            return Err("finish or cancel the sketch first".into());
        }
        if self.solving() {
            return Err("a solve is running (Esc cancels it)".into());
        }
        Ok(())
    }

    /// Replace the whole model (undoable) and forget everything that belonged to the old one.
    pub(super) fn start_model(&mut self, problem: Problem) {
        self.replace(problem);
        self.selected_feature = None;
        self.solved = None;
        self.preview = None;
        self.preview_error = None;
        self.mesh_requested = false;
        self.show_results = false;
        self.show_study = false;
        self.study = None;
        self.pick_positions.clear();
        self.touch();
    }

    /// File > New: an empty part with an empty Sketch1 on the Front plane, opened for drawing.
    pub(super) fn new_part(&mut self) {
        if let Err(e) = self.replace_availability() {
            self.status = e;
            return;
        }
        let blank = fea_problem::templates::blank();
        let feature = blank.history.as_ref().and_then(|h| h.features.first()).map(|f| f.id);
        self.start_model(blank.clone());
        self.dirty = false;
        if let Some(id) = feature {
            self.enter_sketch(id, blank);
        }
        self.status = "New part: draw Sketch1 and Finish it, then extrude it or mesh it (Undo brings the previous model back).".into();
    }

    /// File > Clear model: nothing left but an empty history; sketches can be added afterwards.
    pub(super) fn clear_model(&mut self) {
        if let Err(e) = self.replace_availability() {
            self.status = e;
            return;
        }
        let mut p = fea_problem::templates::blank();
        p.history = Some(fea_problem::history::History::default());
        self.start_model(p);
        self.status = "Model cleared (Undo brings it back). Add a sketch to start again.".into();
    }

    pub(super) fn open_template(&mut self, index: usize) {
        if let Err(e) = self.replace_availability() {
            self.status = e;
            return;
        }
        match templates().into_iter().nth(index) {
            Some((title, problem)) => {
                self.template = index;
                self.start_model(problem);
                self.status = format!("Started from template '{title}' (undoable).");
            }
            None => self.status = "No such template.".into(),
        }
    }

    pub(super) fn begin_open_file(&mut self) {
        if let Err(e) = self.replace_availability() {
            self.status = e;
            return;
        }
        let text = self.path.to_string_lossy().into_owned();
        self.prompt = Some(Prompt { kind: EditKind::Text, label: "open problem (.json), mesh (.msh / .inp) or STEP solid (.step / .stp)".into(), text, row: None, open: true, op: None });
        self.status = "Type the path and press Enter (Esc cancels).".into();
        self.touch();
    }

    /// Open a saved problem (`.json`) or an imported mesh (anything else) at `path`.
    pub fn open_path(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }
        if let Err(e) = self.replace_availability() {
            self.status = e;
            return;
        }
        let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "step" | "stp") {
            self.open_step(path);
            return;
        }
        let is_json = Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
        if is_json {
            match read_problem(Path::new(path)) {
                Ok(mut p) => {
                    let history_error = p.apply_history().err();
                    self.start_model(p);
                    self.path = PathBuf::from(path);
                    self.dirty = false;
                    self.status = match history_error {
                        Some(e) => format!("Opened {path}, but its feature history cannot be rebuilt: {e}"),
                        None => format!("Opened {path}"),
                    };
                }
                Err(e) => self.status = format!("Open failed: {e}"),
            }
        } else {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    self.start_model(fea_problem::templates::imported(path));
                    self.import = Some((path.to_string(), text));
                    self.status = format!("Opened mesh {path}; set the analysis type to match it, then Generate mesh.");
                }
                Err(e) => self.status = format!("Could not read {path}: {e}"),
            }
        }
    }

    /// What to offer over an empty viewport: a new part, each template, a file.
    pub(super) fn start_entries(&self) -> Vec<Entry> {
        if self.sketch_mode() || !self.problem.history.as_ref().is_some_and(|h| h.features.is_empty()) {
            return Vec::new();
        }
        let when = self.replace_availability();
        let mut out = vec![Entry::new("New part", "Ctrl/Cmd+N", ui(SolidAction::NewPart)).when(when.clone())];
        out.extend(templates().into_iter().enumerate().map(|(i, (title, _))| Entry::new(title, "", ui(SolidAction::OpenTemplate(i))).when(when.clone())));
        out.push(Entry::new("Open problem or mesh file...", "Ctrl/Cmd+O", ui(SolidAction::OpenFile)).when(when));
        out
    }

    /// The File-menu block that replaces the model: New, Open, one entry per template and Clear.
    pub(super) fn model_items(&self) -> Vec<Item> {
        let when = self.replace_availability();
        let mut out = vec![
            Item::Entry(Entry::new("New part", "Ctrl/Cmd+N", ui(SolidAction::NewPart)).when(when.clone())),
            Item::Entry(Entry::new("Open problem or mesh file...", "Ctrl/Cmd+O", ui(SolidAction::OpenFile)).when(when.clone())),
            Item::Separator,
            Item::Heading("Templates".into()),
        ];
        for (i, (title, _)) in templates().into_iter().enumerate() {
            out.push(Item::Entry(Entry::new(title, "", ui(SolidAction::OpenTemplate(i))).when(when.clone()).checked(self.template == i && !self.dirty)));
        }
        out.push(Item::Separator);
        out.push(Item::Entry(Entry::new("Clear model", "", ui(SolidAction::ClearModel)).when(when)));
        out.push(Item::Separator);
        for kind in ExportKind::ALL {
            out.push(Item::Entry(Entry::new(kind.label(), "", ui(SolidAction::Export(kind))).when(self.results_availability())));
        }
        out
    }
}
