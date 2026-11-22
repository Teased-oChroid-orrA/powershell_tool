//! The Assembly Joints window: a STEP assembly read body by body (`fea_problem::assembly_mesh`), the holes, fastener sites and contact
//! interfaces found in it (`solid_kernel::features`) and the advice over them (`fea_problem::assembly_advice`), with the faces of the
//! selected item or body highlighted on the model. Epic `docs/epic-assembly-feature-advice.md`, Phase B.
//!
//! Read-only: nothing is applied to a `Problem` (it holds one body) and nothing is written. The window only reads the file it was
//! given; File > Open picks another. The model is the viewer mesh, so a body is its surface, not an analysis mesh.
mod cards;
pub mod scene;
#[cfg(test)]
mod tests;

use super::{
    authoring::{Authoring, Card, StepState},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::gpu_viewer::{
    bench::Bench,
    controller::StandardView,
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
    Scene,
};
use fea_problem::assembly_advice::{recommend, AssemblyAdvice};
use fea_problem::assembly_mesh::{read_assembly, AssemblyMesh, AssemblyRead, FaceKey, DEFAULT_CHORD};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// What is highlighted on the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sel {
    /// An advice item (its faces).
    Item(usize),
    /// A whole body.
    Body(usize),
}

/// What a click in the Assembly Joints window asks for (carried by `EditorAction::Assembly`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AssemblyAct {
    Select(Sel),
    ClearSelection,
    ToggleExploded,
    /// Pick another STEP file.
    Open,
}

/// Everything read from the file.
pub struct Loaded {
    pub mesh: AssemblyMesh,
    pub advice: Vec<AssemblyAdvice>,
    pub holes: usize,
    pub holes_per_body: Vec<usize>,
    pub sites: usize,
    pub interfaces: usize,
    /// Bolt-sized holes with no mate on another body (the `Unmated` advice items).
    pub unmated: usize,
    pub unresolved: usize,
    /// Bores that cover nearly a full turn (the `NearClosed` advice items): not holes, a candidate modelling gap shown to the person.
    pub near_closed: usize,
    /// Seconds to read, detect and mesh.
    pub seconds: f64,
}

pub struct AssemblySession {
    path: PathBuf,
    name: String,
    loaded: Option<Loaded>,
    selected: Option<Sel>,
    exploded: bool,
    status: String,
    /// The picture is rebuilt only after a change.
    drawn: bool,
}

/// Read, detect and mesh the STEP assembly at `path`.
pub fn load(path: &Path) -> Result<Loaded, String> {
    let started = std::time::Instant::now();
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    // one parse and one read of the bodies serve both the detection and the picture
    let AssemblyRead { features, mesh } = read_assembly(&text, DEFAULT_CHORD)?;
    let mut holes_per_body = vec![0; mesh.bodies()];
    for h in &features.holes {
        if let Some(n) = holes_per_body.get_mut(h.body) {
            *n += 1;
        }
    }
    let advice = recommend(&features);
    Ok(Loaded {
        unmated: advice.iter().filter(|a| matches!(a, AssemblyAdvice::Unmated { .. })).count(),
        advice,
        holes: features.holes.len(),
        holes_per_body,
        sites: features.sites.len(),
        interfaces: features.interfaces.len(),
        unresolved: features.unresolved.len(),
        near_closed: features.near_closed.len(),
        mesh,
        seconds: started.elapsed().as_secs_f64(),
    })
}

impl AssemblySession {
    /// A session on the STEP file `path`; an empty path or a file that cannot be read opens the empty window with the reason in the
    /// status bar, so File > Open can pick another.
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut s = Self { path: path.to_path_buf(), name: String::new(), loaded: None, selected: None, exploded: true, status: String::new(), drawn: false };
        if !path.as_os_str().is_empty() {
            s.load_path(path.to_path_buf());
        }
        Ok(s)
    }

    pub fn loaded(&self) -> Option<&Loaded> {
        self.loaded.as_ref()
    }
    pub fn selection(&self) -> Option<Sel> {
        self.selected
    }
    pub fn exploded(&self) -> bool {
        self.exploded
    }

    fn load_path(&mut self, path: PathBuf) {
        self.status.clear();
        match load(&path) {
            Ok(l) => {
                self.name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("assembly").to_string();
                self.path = path;
                self.loaded = Some(l);
                self.selected = None;
            }
            Err(e) => self.status = format!("Cannot open {}: {e}", path.display()),
        }
        self.drawn = false;
    }

    /// The faces the current selection highlights.
    pub fn selected_faces(&self) -> Vec<FaceKey> {
        let Some(l) = &self.loaded else { return Vec::new() };
        match self.selected {
            Some(Sel::Item(i)) => l.advice.get(i).map(|a| a.faces()).unwrap_or_default(),
            // each face once, in key order (`dedup` only drops neighbours, and nothing orders the triangles by face)
            Some(Sel::Body(b)) => l.mesh.face.iter().filter(|k| k.body == b).copied().collect::<BTreeSet<_>>().into_iter().collect(),
            None => Vec::new(),
        }
    }

    fn select(&mut self, s: Option<Sel>) {
        if self.selected != s {
            self.selected = s;
            self.drawn = false;
        }
    }
    /// Move the selection to the next / previous advice item.
    fn step_item(&mut self, forward: bool) {
        let Some(l) = &self.loaded else { return };
        let n = l.advice.len();
        if n == 0 {
            return;
        }
        let next = match self.selected {
            Some(Sel::Item(i)) => if forward { (i + 1) % n } else { (i + n - 1) % n },
            _ => if forward { 0 } else { n - 1 },
        };
        self.select(Some(Sel::Item(next)));
    }

    fn do_act(&mut self, a: AssemblyAct) {
        self.status.clear();
        match a {
            AssemblyAct::Select(s) => self.select(Some(s)),
            AssemblyAct::ClearSelection => self.select(None),
            AssemblyAct::ToggleExploded => {
                self.exploded = !self.exploded;
                self.drawn = false;
            }
            AssemblyAct::Open => self.pick_file(),
        }
    }

    fn pick_file(&mut self) {
        #[cfg(feature = "gpu-viewer")]
        {
            let start = self.path.parent().map(Path::to_path_buf).filter(|p| p.is_dir()).unwrap_or_default();
            if let Some(path) = rfd::FileDialog::new().set_title("Open a STEP assembly").add_filter("STEP", &["step", "stp"]).set_directory(start).pick_file() {
                self.load_path(path);
            }
        }
        #[cfg(not(feature = "gpu-viewer"))]
        {
            self.status = "rebuild with the gpu-viewer feature to pick a file".into();
        }
    }
}

impl Authoring for AssemblySession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn key_labels(&self) -> [&'static str; 4] {
        ["Bodies 1, 3, 5", "Bodies 2, 4, 6", "Not used", "Selected"]
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::Assembly(a) = action {
            self.do_act(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        match key {
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'o') => self.do_act(AssemblyAct::Open),
            Key::Char(c) if !mods.command && c.eq_ignore_ascii_case(&'e') => self.do_act(AssemblyAct::ToggleExploded),
            Key::Down => self.step_item(true),
            Key::Up => self.step_item(false),
            Key::Escape if self.selected.is_some() => self.select(None),
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    // Picking a face on the model is not offered in this step: the list and the tree select.
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        self.drawn = true;
        let Some(l) = &self.loaded else { return Ok(crate::gpu_viewer::editor::drawing::empty("assembly".into(), [0.0, 3.0])) };
        let faces: HashSet<FaceKey> = self.selected_faces().into_iter().collect();
        let label = format!("{} | {} bodies: blue odd, teal even, red selected{}", self.name, l.mesh.bodies(), if self.exploded { " | exploded" } else { "" });
        scene::scene(&l.mesh, self.exploded, |k| faces.contains(&k), label)
    }
    fn needs_scene(&self) -> bool {
        !self.drawn
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
        if !self.status.is_empty() {
            return self.status.clone();
        }
        self.summary()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn title(&self) -> String {
        if self.name.is_empty() { "Assembly Joints".into() } else { format!("Assembly Joints | {}", self.name) }
    }
    fn dirty(&self) -> bool {
        false
    }
    fn initial_view(&self) -> Option<StandardView> {
        Some(StandardView::Isometric)
    }
    fn shows_body_only(&self) -> bool {
        true
    }
    fn focus_step(&self) -> Option<&'static str> {
        Some("Assembly")
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
    fn step_state(&self, tab: &str) -> Option<StepState> {
        (tab == "Assembly").then(|| if self.loaded.is_some() { StepState::Done } else { StepState::Todo })
    }
    fn start_options(&self) -> Vec<Entry> {
        if self.loaded.is_some() { Vec::new() } else { vec![cards::entry("Open STEP assembly", "Ctrl/Cmd+O", AssemblyAct::Open)] }
    }
    fn menus(&self) -> Vec<Menu> {
        self.menu_bar()
    }
    fn context_menu(&self) -> Vec<Item> {
        self.menu_bar().into_iter().find(|m| m.title == "View").map(|m| m.items).unwrap_or_default()
    }
}
