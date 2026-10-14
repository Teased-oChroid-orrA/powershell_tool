//! What the native window needs from an interactive model editor, independent of the model kind
//! (frame/truss members, meshed solids). `window.rs` and `chrome.rs` only talk to this trait; each
//! session type supplies its own tree, property manager, ribbon and menus so availability rules stay
//! with the model that owns them.
use super::{
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::gpu_viewer::{
    menu::{Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
    Scene,
};

/// Where one workflow step (a ribbon tab) stands: the thin step bar marks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    /// Not started, or out of date.
    Todo,
    Done,
}

/// A label the editor wants drawn on the model in the viewport (a sketch's dimension value, a constraint glyph).
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    /// Viewport pixel (origin at the viewport's top-left).
    pub px: [f32; 2],
    pub text: String,
    pub kind: OverlayKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// A driving dimension: a boxed value.
    Dimension,
    /// A geometric constraint: a small glyph.
    Constraint,
}

/// How much a card of the inspector matters right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relevance {
    /// What the person is working on (the selected entity's values, a prompt being typed).
    Primary,
    /// Help for the current step (mesh density, recommendations).
    Context,
    /// Status and results.
    Info,
}

/// One titled block of the inspector. The editor says what is relevant; the window decides what to show, in what
/// order and how open, and learns that from use (`adapt`).
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// Stable across frames and sessions (what the adaptive layout remembers).
    pub id: String,
    pub title: String,
    pub relevance: Relevance,
    /// Steps (ribbon tabs) this card belongs to; empty: every step.
    pub steps: Vec<&'static str>,
    pub items: Vec<DockItem>,
}
impl Card {
    pub fn new(id: impl Into<String>, title: impl Into<String>, relevance: Relevance, items: Vec<DockItem>) -> Self {
        Self { id: id.into(), title: title.into(), relevance, steps: Vec::new(), items }
    }
    pub fn for_steps(mut self, steps: &[&'static str]) -> Self {
        self.steps = steps.to_vec();
        self
    }
}

/// Split a flat property list at its headings into cards: the first is primary, the rest context. Gaps only
/// separated sections in the old fixed dock and are dropped.
pub fn cards_from(items: Vec<DockItem>) -> Vec<Card> {
    let slug = |t: &str| t.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-");
    let mut cards: Vec<Card> = Vec::new();
    for item in items {
        match item {
            DockItem::Heading(h) => cards.push(Card::new(slug(&h), h, Relevance::Context, Vec::new())),
            DockItem::Gap => {}
            other => {
                if cards.is_empty() {
                    cards.push(Card::new("properties", "Properties", Relevance::Context, Vec::new()));
                }
                cards.last_mut().expect("just ensured").items.push(other);
            }
        }
    }
    cards.retain(|c| !c.items.is_empty());
    if let Some(first) = cards.first_mut() {
        first.relevance = Relevance::Primary;
    }
    cards
}

/// The cards as one flat list (title heading, then the rows).
pub fn flatten(cards: Vec<Card>) -> Vec<DockItem> {
    cards.into_iter().flat_map(|c| std::iter::once(DockItem::Heading(c.title)).chain(c.items)).collect()
}

pub trait Authoring {
    /// The bench this session is showing.
    fn bench(&self) -> crate::gpu_viewer::bench::Bench;
    /// File the model is saved to (sibling benches' files live beside it).
    fn path(&self) -> &std::path::Path;
    /// Switch to another bench of the same model kind in place; false when the model kind differs.
    fn switch_bench(&mut self, _bench: crate::gpu_viewer::bench::Bench) -> bool {
        false
    }
    fn act(&mut self, action: EditorAction);
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult;
    /// Left click on the model at a window pixel (no drag). True when the editor used it.
    fn click(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool;
    /// Click with the add-to-selection modifier (Shift): toggles instead of replacing.
    fn click_with(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) -> bool {
        let _ = additive;
        self.click(px, size, cam)
    }
    /// Cursor moved over the model without a button: true when the editor wants a redraw (hover preview).
    fn hover(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    /// Left press over the model: true when the editor takes the gesture (it then gets `drag` / `release_grab`
    /// instead of the view orbiting).
    fn grab(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn drag(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn release_grab(&mut self) {}
    /// A camera orientation the editor asks for once (entering a sketch looks at its plane).
    fn take_view(&mut self) -> Option<crate::gpu_viewer::controller::StandardView> {
        None
    }
    /// Shift-drag box (viewport pixels): select everything whose centre is inside.
    fn box_select(&mut self, _a: [f32; 2], _b: [f32; 2], _size: [f32; 2], _cam: &Camera, _additive: bool) {}
    /// Right press: select what is under the cursor without applying a tool.
    fn select_at(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera);
    /// Scene for the current state; the placeholder for a model that cannot be drawn yet.
    fn scene(&mut self) -> Result<Scene, String>;
    fn needs_scene(&self) -> bool;
    /// Collect finished background work; true when something changed.
    fn poll(&mut self) -> bool;
    /// A number or text is being typed: every key belongs to it.
    fn prompting(&self) -> bool;
    fn solving(&self) -> bool;
    fn status(&self) -> String;
    /// A one-click fix for the current failure, offered as a button in the status bar (label and the action it runs).
    fn status_fix(&self) -> Option<(String, crate::gpu_viewer::controller::UiAction)> {
        None
    }
    fn set_status(&mut self, text: String);
    fn title(&self) -> String;
    fn dirty(&self) -> bool;
    /// Camera orientation when the window opens (`None` keeps the front view).
    fn initial_view(&self) -> Option<crate::gpu_viewer::controller::StandardView> {
        None
    }
    /// The scene is the model's body, not an analysis mesh: the mesh lines would draw its stand-in tessellation.
    fn shows_body_only(&self) -> bool {
        false
    }
    /// The form to show beside the model (a constraint's directions, a load's numbers), if one is open.
    fn value_card(&self) -> Option<crate::gpu_viewer::ui::ValueCard> {
        None
    }
    /// The text of field `index` of the value card changed.
    fn card_field(&mut self, _index: usize, _text: String) {}
    /// Draw element edges when the window opens.
    fn default_mesh_lines(&self) -> bool {
        false
    }

    /// Feature tree of the model.
    fn tree(&self) -> Vec<TreeNode>;
    /// Property rows for the current selection as one flat list (what `inspector` is made of, for tests and logs).
    fn properties(&self) -> Vec<DockItem>;
    /// The inspector: what matters for the current selection and step, as cards. The window shows only the relevant
    /// ones, floating over the model, and adapts to how they are used.
    fn inspector(&self) -> Vec<Card> {
        cards_from(self.properties())
    }
    /// Right-click menu of a feature-tree row: the choices and actions that belong to that entity (the value fields
    /// stay in the inspector).
    fn tree_menu(&self, _node: u32) -> Vec<Item> {
        Vec::new()
    }
    /// The camera and viewport size (pixels) of the frame about to be drawn, so geometry whose size is meant to
    /// be constant on the screen (sketch lines) can follow the zoom. Called every frame.
    fn set_view(&mut self, _cam: &Camera, _size: [f32; 2]) {}
    /// Labels to draw on the model for the current camera (`size` is the viewport in pixels).
    fn overlay(&self, _cam: &Camera, _size: [f32; 2]) -> Vec<Overlay> {
        Vec::new()
    }
    /// The step (ribbon tab) the current selection belongs to, so selecting a row of the tree brings that step's tools.
    fn focus_step(&self) -> Option<&'static str> {
        None
    }
    /// How long the running solve has been going.
    fn solving_for(&self) -> Option<std::time::Duration> {
        None
    }
    /// Ribbon tabs specific to this model kind (the viewer adds its own View tab).
    fn ribbon(&self) -> Vec<RibbonTab>;
    /// How to begin when there is nothing to work on yet (an empty model): shown as a card over the viewport.
    fn start_options(&self) -> Vec<crate::gpu_viewer::menu::Entry> {
        Vec::new()
    }
    /// State of the workflow step shown by ribbon tab `tab` (`None`: the tab is not a step of this model).
    fn step_state(&self, _tab: &str) -> Option<StepState> {
        None
    }
    /// Menus specific to this model kind, by title: File, Edit, Insert, Tools, Simulation.
    fn menus(&self) -> Vec<Menu>;
    /// A list the last click asked for (applicable supports / loads of the clicked entity), shown at the cursor.
    fn take_popup(&mut self) -> Option<Vec<Item>> {
        None
    }
    /// Right-click menu content for the current selection.
    fn context_menu(&self) -> Vec<Item>;
}
