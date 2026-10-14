//! Pure viewer controller: display state and the commands that change it. The winit runner only
//! maps input to `Command`; nothing here touches the window or the GPU, so every operation is
//! unit-testable and a future in-window panel or editor can issue the same commands.
use super::{
    display::{Background, DisplayStyle},
    Scene,
};

pub const DEFAULT_ZOOM: f32 = 1.6;
pub const MIN_ZOOM: f32 = 0.1;
/// Reaches 250x the default fit, enough to resolve a 0.001 in feature of a 10 in sketch.
pub const MAX_ZOOM: f32 = 400.0;
/// How far the view may be shifted off the viewport (NDC units; 2 = one viewport).
pub const PAN_LIMIT: f32 = 400.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    TogglePause,
    Rewind,
    ToggleMesh,
    ToggleContour,
    ToggleDeformation,
    ResetCamera,
    /// Pause and move one stored frame (or 1/120 of a harmonic period) forward or back.
    StepFrame { forward: bool },
    GainUp,
    GainDown,
    SpeedUp,
    SpeedDown,
    /// Camera orbit by cursor movement in pixels.
    Orbit { dx: f32, dy: f32 },
    /// Zoom by wheel lines.
    Zoom(f32),
    /// Shift the view on the screen by an offset in NDC units (2 = the viewport's width or height).
    Pan { dx: f32, dy: f32 },
    /// Select the next/previous scalar field of the active result (wraps).
    CycleField { forward: bool },
    ToggleHelp,
    /// Snap the camera to a standard orientation.
    StandardView(StandardView),
    /// Select scalar field `n` (0 = the primary one); a stale index falls back to the primary.
    SetField(usize),
    /// Draw the model as shaded, with edges, hidden lines visible / removed, or wireframe.
    SetStyle(DisplayStyle),
    /// The viewport background.
    SetBackground(Background),
}

/// CAD standard views, defined by where the viewer stands: front at -z (x right, y up on the screen),
/// back +z, left -x, right +x, top +y (z up on the screen), bottom -y.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Isometric,
}
impl StandardView {
    pub const ALL: [StandardView; 7] =
        [StandardView::Front, StandardView::Back, StandardView::Left, StandardView::Right, StandardView::Top, StandardView::Bottom, StandardView::Isometric];
    pub fn label(self) -> &'static str {
        match self {
            StandardView::Front => "Front",
            StandardView::Back => "Back",
            StandardView::Left => "Left",
            StandardView::Right => "Right",
            StandardView::Top => "Top",
            StandardView::Bottom => "Bottom",
            StandardView::Isometric => "Isometric",
        }
    }
    /// `(yaw, pitch)` in radians.
    pub fn angles(self) -> (f32, f32) {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        match self {
            StandardView::Front => (0.0, 0.0),
            StandardView::Back => (PI, 0.0),
            StandardView::Left => (-FRAC_PI_2, 0.0),
            StandardView::Right => (FRAC_PI_2, 0.0),
            StandardView::Top => (0.0, -FRAC_PI_2),
            StandardView::Bottom => (0.0, FRAC_PI_2),
            StandardView::Isometric => (-FRAC_PI_4, -0.6155),
        }
    }
}


/// What a click on a HUD row does. Result switching needs the renderer, so the runner handles it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiAction {
    Command(Command),
    CycleResult { forward: bool },
    /// Open another bench of the CAD window (see `bench`).
    SwitchBench(super::bench::Bench),
    /// Open the command palette (every menu entry, searchable).
    OpenPalette,
    /// A click on an editor-panel row (see `editor::session`).
    Editor(super::editor::session::EditorAction),
    /// Show / hide a docked panel (Window menu).
    TogglePanel(Panel),
    /// Make result `n` the active one (feature tree).
    SelectResult(usize),
    /// Adaptive layout controls (Window menu).
    Layout(LayoutOp),
    /// Open or close the background colour picker (View menu).
    BackgroundPicker,
    /// Close the window.
    Quit,
}

/// What the Window menu can ask of the adaptive layout (ADR-018).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutOp {
    /// Learn from use, or keep the fixed layout.
    ToggleAdaptive,
    /// Forget what was learned.
    Reset,
    /// Say what has been learned.
    Explain,
    /// Open or fold the feature tree.
    ToggleTree,
}

/// The dockable regions of the window chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Ribbon,
    Tree,
    Properties,
    Status,
}

/// Which optional regions of the window are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visible {
    pub ribbon: bool,
    pub tree: bool,
    pub properties: bool,
    pub status: bool,
}
impl Default for Visible {
    fn default() -> Self {
        Self { ribbon: true, tree: true, properties: true, status: true }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    pub show_mesh: bool,
    pub show_contour: bool,
    pub deformed: bool,
    pub paused: bool,
    pub time: f32,
    pub speed: f32,
    pub gain: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    /// Screen-space shift of the view in NDC units.
    pub pan: [f32; 2],
    /// 0 = the scene's primary scalar, k = `scene.fields[k - 1]`.
    pub field: usize,
    pub show_help: bool,
    pub style: DisplayStyle,
    pub background: Background,
}

impl ViewState {
    pub fn new(scene: &Scene) -> Self {
        Self {
            show_mesh: false,
            show_contour: true,
            deformed: true,
            paused: false,
            time: 0.0,
            speed: default_speed(scene),
            gain: scene.gain,
            yaw: 0.0,
            pitch: 0.0,
            zoom: DEFAULT_ZOOM,
            pan: [0.0, 0.0],
            field: 0,
            show_help: false,
            style: DisplayStyle::default(),
            background: Background::default(),
        }
    }

    /// Reset the per-result playback state when another computed result becomes active. Display
    /// toggles and the camera persist across results.
    pub fn enter_scene(&mut self, scene: &Scene) {
        self.gain = scene.gain;
        self.time = 0.0;
        self.speed = default_speed(scene);
        self.field = 0;
    }

    /// The style actually drawn: a result is always painted with its contour, so a style without a surface shows it
    /// as shaded with edges. The chosen style is kept and returns with the model.
    pub fn style_for(&self, scene: &Scene) -> DisplayStyle {
        if !scene.fields.is_empty() && !self.style.paints_surface() {
            DisplayStyle::ShadedEdges
        } else {
            self.style
        }
    }

    /// Colour range of the selected field (falls back to the primary if the index is stale).
    pub fn range(&self, scene: &Scene) -> [f32; 2] {
        match self.field.checked_sub(1).and_then(|k| scene.fields.get(k)) {
            Some(f) => f.range,
            None => scene.range,
        }
    }

    pub fn field_name(&self, scene: &Scene) -> String {
        match self.field.checked_sub(1).and_then(|k| scene.fields.get(k)) {
            Some(f) => f.label.clone(),
            None if scene.field_label.is_empty() => "result value".into(),
            None => scene.field_label.clone(),
        }
    }

    /// Wrapping index of the next/previous result; `count` must be non-zero.
    pub fn cycle(selected: usize, count: usize, forward: bool) -> usize {
        if forward {
            (selected + 1) % count
        } else {
            (selected + count - 1) % count
        }
    }

    /// Advance playback by `dt` seconds of wall time (no-op when paused or static).
    pub fn advance(&mut self, scene: &Scene, dt: f32) {
        let end = scene.duration();
        if !self.paused && end > 0.0 {
            self.time = (self.time + dt * self.speed) % end;
        }
    }

    pub fn apply(&mut self, command: Command, scene: &Scene) {
        match command {
            Command::TogglePause => self.paused = !self.paused,
            Command::Rewind => self.time = 0.0,
            Command::ToggleMesh => self.show_mesh = !self.show_mesh,
            Command::SetStyle(style) => self.style = style,
            Command::SetBackground(background) => self.background = background,
            Command::ToggleContour => self.show_contour = !self.show_contour,
            Command::ToggleDeformation => self.deformed = !self.deformed,
            Command::ResetCamera => {
                self.yaw = 0.0;
                self.pitch = 0.0;
                self.zoom = DEFAULT_ZOOM;
                self.pan = [0.0, 0.0];
            }
            Command::StepFrame { forward } => {
                self.paused = true;
                if let Some(period) = scene.harmonic_period {
                    let delta = if forward { period / 120.0 } else { -period / 120.0 };
                    self.time = (self.time + delta).rem_euclid(period);
                } else if !scene.times.is_empty() {
                    let i = scene
                        .times
                        .partition_point(|t| *t <= self.time)
                        .saturating_sub(1);
                    let j = if forward {
                        (i + 1).min(scene.times.len() - 1)
                    } else {
                        i.saturating_sub(1)
                    };
                    self.time = scene.times[j];
                }
            }
            Command::GainUp => self.gain = (self.gain * 1.25).min(1e12),
            Command::GainDown => self.gain /= 1.25,
            Command::SpeedUp => self.speed = (self.speed * 2.0).min(1024.0),
            Command::SpeedDown => self.speed = (self.speed / 2.0).max(1.0 / 1024.0),
            Command::Orbit { dx, dy } => {
                self.yaw += dx * 0.01;
                self.pitch = (self.pitch + dy * 0.01).clamp(-1.5, 1.5);
            }
            Command::CycleField { forward } => {
                let count = 1 + scene.fields.len();
                self.field = Self::cycle(self.field.min(count - 1), count, forward);
            }
            Command::ToggleHelp => self.show_help = !self.show_help,
            Command::StandardView(v) => {
                (self.yaw, self.pitch) = v.angles();
            }
            Command::SetField(n) => self.field = if n <= scene.fields.len() { n } else { 0 },
            Command::Zoom(lines) => self.zoom = (self.zoom * (1.0 + lines * 0.1)).clamp(MIN_ZOOM, MAX_ZOOM),
            Command::Pan { dx, dy } => self.pan = [(self.pan[0] + dx).clamp(-PAN_LIMIT, PAN_LIMIT), (self.pan[1] + dy).clamp(-PAN_LIMIT, PAN_LIMIT)],
        }
    }
}

/// Left-button gesture tracking: a press that travels `DRAG_PIXELS` becomes an orbit, otherwise its
/// release is a click. Positions are physical window pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pointer {
    pub cursor: Option<(f64, f64)>,
    press: Option<(f64, f64)>,
    dragging: bool,
    /// The gesture is a selection box (Shift held at the press), not an orbit.
    boxing: bool,
}
/// Cursor travel before a press becomes an orbit instead of a click.
pub const DRAG_PIXELS: f64 = 4.0;
impl Pointer {
    /// Start a gesture at the current cursor unless a HUD panel consumed the press.
    pub fn press(&mut self, consumed_by_hud: bool) {
        self.press_as(consumed_by_hud, false);
    }
    /// As [`press`](Self::press); `boxing` makes a drag a selection box instead of an orbit.
    pub fn press_as(&mut self, consumed_by_hud: bool, boxing: bool) {
        self.dragging = false;
        self.boxing = boxing;
        self.press = if consumed_by_hud { None } else { self.cursor };
    }
    /// The box being dragged (press point, cursor) once the gesture is a drag in box mode.
    pub fn box_drag(&self) -> Option<((f64, f64), (f64, f64))> {
        Some((self.press?, self.cursor?)).filter(|_| self.boxing && self.dragging)
    }
    /// Cursor moved: `Some(Orbit)` command once the gesture is a drag.
    pub fn moved(&mut self, to: (f64, f64)) -> Option<Command> {
        let previous = self.cursor.replace(to);
        let (start, (x, y)) = (self.press?, previous?);
        if !self.dragging && (to.0 - start.0).hypot(to.1 - start.1) > DRAG_PIXELS {
            self.dragging = true;
        }
        (self.dragging && !self.boxing).then_some(Command::Orbit { dx: (to.0 - x) as f32, dy: (to.1 - y) as f32 })
    }
    /// Button released: the click position when the gesture never became a drag.
    pub fn release(&mut self) -> Option<(f64, f64)> {
        let click = self.press.take().filter(|_| !self.dragging);
        self.dragging = false;
        click
    }
}

/// A pan gesture: the middle button, or the right button (whose release without travel is a click that
/// opens the context menu). Positions are physical window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanGesture {
    start: (f64, f64),
    dragging: bool,
}
impl PanGesture {
    /// A middle-button press pans at once; a right-button press only after `DRAG_PIXELS` of travel.
    pub fn begin(at: (f64, f64), immediate: bool) -> Self {
        Self { start: at, dragging: immediate }
    }
    /// Cursor moved from `from` to `to`: the pixel offset to pan by once the gesture is a drag.
    pub fn moved(&mut self, from: (f64, f64), to: (f64, f64)) -> Option<(f64, f64)> {
        if !self.dragging && (to.0 - self.start.0).hypot(to.1 - self.start.1) > DRAG_PIXELS {
            self.dragging = true;
            // Catch up with the travel so far, so the model follows the cursor from where it was pressed.
            return Some((to.0 - self.start.0, to.1 - self.start.1));
        }
        self.dragging.then_some((to.0 - from.0, to.1 - from.1))
    }
    /// Button released: true when the gesture never became a drag (a click).
    pub fn is_click(&self) -> bool {
        !self.dragging
    }
}

fn default_speed(scene: &Scene) -> f32 {
    if scene.duration() > 0.0 {
        scene.duration() / 4.0
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(times: Vec<f32>, period: Option<f32>) -> Scene {
        Scene {
            label: "t".into(),
            positions: vec![],
            triangles: vec![],
            samples: vec![],
            times,
            range: [0.0, 1.0],
            gain: 2.0,
            harmonic_period: period,
            field_label: String::new(),
            fields: Vec::new(),
        }
    }

    #[test]
    fn cycle_wraps_both_ways() {
        assert_eq!(ViewState::cycle(2, 3, true), 0);
        assert_eq!(ViewState::cycle(0, 3, false), 2);
        assert_eq!(ViewState::cycle(0, 1, true), 0);
        assert_eq!(ViewState::cycle(0, 1, false), 0);
    }

    #[test]
    fn step_frame_walks_stored_times_and_pauses() {
        let s = scene(vec![0.0, 1.0, 2.0], None);
        let mut v = ViewState::new(&s);
        v.apply(Command::StepFrame { forward: true }, &s);
        assert!(v.paused);
        assert_eq!(v.time, 1.0);
        v.apply(Command::StepFrame { forward: true }, &s);
        v.apply(Command::StepFrame { forward: true }, &s);
        assert_eq!(v.time, 2.0, "clamps at the last frame");
        v.apply(Command::StepFrame { forward: false }, &s);
        assert_eq!(v.time, 1.0);
        v.time = 0.0;
        v.apply(Command::StepFrame { forward: false }, &s);
        assert_eq!(v.time, 0.0, "clamps at the first frame");
    }

    #[test]
    fn harmonic_step_wraps_within_period() {
        let s = scene(vec![0.0], Some(1.2));
        let mut v = ViewState::new(&s);
        v.apply(Command::StepFrame { forward: false }, &s);
        assert!((v.time - (1.2 - 0.01)).abs() < 1e-6);
        v.apply(Command::StepFrame { forward: true }, &s);
        assert!(v.time.abs() < 1e-6 || (v.time - 1.2).abs() < 1e-6);
    }

    #[test]
    fn step_frame_on_empty_times_is_safe() {
        let s = scene(vec![], None);
        let mut v = ViewState::new(&s);
        v.apply(Command::StepFrame { forward: true }, &s);
        assert_eq!(v.time, 0.0);
    }

    #[test]
    fn limits_and_camera() {
        let s = scene(vec![0.0, 1.0], None);
        let mut v = ViewState::new(&s);
        assert_eq!(v.speed, 0.25);
        for _ in 0..40 {
            v.apply(Command::SpeedUp, &s);
        }
        assert_eq!(v.speed, 1024.0);
        for _ in 0..40 {
            v.apply(Command::SpeedDown, &s);
        }
        assert_eq!(v.speed, 1.0 / 1024.0);
        v.apply(Command::Orbit { dx: 50.0, dy: 1e6 }, &s);
        assert_eq!(v.pitch, 1.5);
        assert!((v.yaw - 0.5).abs() < 1e-6);
        for _ in 0..200 {
            v.apply(Command::Zoom(5.0), &s);
        }
        assert_eq!(v.zoom, MAX_ZOOM);
        for _ in 0..400 {
            v.apply(Command::Zoom(-5.0), &s);
        }
        assert_eq!(v.zoom, MIN_ZOOM);
        for _ in 0..50 {
            v.apply(Command::Pan { dx: 100.0, dy: -100.0 }, &s);
        }
        assert_eq!(v.pan, [PAN_LIMIT, -PAN_LIMIT], "the pan is bounded");
        v.apply(Command::ResetCamera, &s);
        assert_eq!((v.yaw, v.pitch, v.zoom, v.pan), (0.0, 0.0, DEFAULT_ZOOM, [0.0, 0.0]));
    }

    #[test]
    fn enter_scene_resets_playback_but_keeps_display_toggles() {
        let a = scene(vec![0.0, 4.0], None);
        let mut b = scene(vec![0.0, 8.0], None);
        b.gain = 7.0;
        let mut v = ViewState::new(&a);
        v.apply(Command::ToggleMesh, &a);
        v.apply(Command::GainUp, &a);
        v.time = 3.0;
        v.enter_scene(&b);
        assert_eq!((v.gain, v.time, v.speed), (7.0, 0.0, 2.0));
        assert!(v.show_mesh);
    }

    #[test]
    fn field_cycle_wraps_and_resets_with_result() {
        let mut a = scene(vec![0.0, 4.0], None);
        a.fields = vec![
            crate::gpu_viewer::scene::ScalarField {
                label: "f1".into(),
                range: [-3.0, 5.0],
                values: vec![],
            },
        ];
        a.field_label = "primary".into();
        let mut v = ViewState::new(&a);
        assert_eq!((v.field_name(&a).as_str(), v.range(&a)), ("primary", [0.0, 1.0]));
        v.apply(Command::CycleField { forward: true }, &a);
        assert_eq!((v.field_name(&a).as_str(), v.range(&a)), ("f1", [-3.0, 5.0]));
        v.apply(Command::CycleField { forward: true }, &a);
        assert_eq!(v.field, 0, "wraps to the primary scalar");
        v.apply(Command::CycleField { forward: false }, &a);
        assert_eq!(v.field, 1);
        let b = scene(vec![0.0, 1.0], None);
        assert_eq!(v.range(&b), [0.0, 1.0], "stale field index falls back to primary");
        assert_eq!(v.field_name(&b), "result value");
        v.apply(Command::CycleField { forward: true }, &b);
        assert_eq!(v.field, 0, "single-field scene stays on the primary");
        v.field = 1;
        v.enter_scene(&b);
        assert_eq!(v.field, 0);
    }
    #[test]
    fn pointer_click_vs_drag() {
        let mut p = Pointer::default();
        assert_eq!(p.moved((10.0, 10.0)), None, "no press: movement does nothing");
        p.press(false);
        assert_eq!(p.moved((12.0, 11.0)), None, "within the dead zone");
        assert_eq!(p.release(), Some((10.0, 10.0)), "a click is reported where the press began");
        // a drag: the first move past the threshold already orbits by the step from the previous position
        p.press(false);
        assert_eq!(p.moved((11.0, 10.0)), None);
        let c = p.moved((20.0, 10.0));
        assert_eq!(c, Some(Command::Orbit { dx: 9.0, dy: 0.0 }));
        assert_eq!(p.moved((20.0, 15.0)), Some(Command::Orbit { dx: 0.0, dy: 5.0 }));
        assert_eq!(p.release(), None, "a drag is not a click");
        // after a drag, movement without a press is inert again
        assert_eq!(p.moved((30.0, 30.0)), None);
        // a press consumed by the HUD never becomes a click or a drag
        p.press(true);
        assert_eq!(p.moved((90.0, 90.0)), None);
        assert_eq!(p.release(), None);
    }
    #[test]
    fn advance_respects_pause_and_wraps() {
        let s = scene(vec![0.0, 4.0], None);
        let mut v = ViewState::new(&s);
        v.speed = 1.0;
        v.advance(&s, 5.0);
        assert!((v.time - 1.0).abs() < 1e-6);
        v.paused = true;
        v.advance(&s, 1.0);
        assert!((v.time - 1.0).abs() < 1e-6);
    }

    #[test]
    fn standard_views_put_the_viewer_on_the_named_side() {
        use crate::gpu_viewer::editor::camera::Camera;
        let cam = |v: StandardView| {
            let (yaw, pitch) = v.angles();
            Camera { yaw, pitch, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] }
        };
        let depth = |v, p: [f32; 3]| cam(v).project(p).1;
        // Nearer = smaller depth. Each view sees its own side of the model first.
        assert!(depth(StandardView::Front, [0.0, 0.0, -1.0]) < depth(StandardView::Front, [0.0, 0.0, 1.0]));
        assert!(depth(StandardView::Back, [0.0, 0.0, 1.0]) < depth(StandardView::Back, [0.0, 0.0, -1.0]));
        assert!(depth(StandardView::Right, [1.0, 0.0, 0.0]) < depth(StandardView::Right, [-1.0, 0.0, 0.0]));
        assert!(depth(StandardView::Left, [-1.0, 0.0, 0.0]) < depth(StandardView::Left, [1.0, 0.0, 0.0]));
        assert!(depth(StandardView::Top, [0.0, 1.0, 0.0]) < depth(StandardView::Top, [0.0, -1.0, 0.0]));
        assert!(depth(StandardView::Bottom, [0.0, -1.0, 0.0]) < depth(StandardView::Bottom, [0.0, 1.0, 0.0]));
        // Screen orientation: front has x right / y up; top has x right / z up.
        let p = |v, q: [f32; 3]| cam(v).project(q).0;
        assert!(p(StandardView::Front, [1.0, 0.0, 0.0])[0] > 0.0 && p(StandardView::Front, [0.0, 1.0, 0.0])[1] > 0.0);
        assert!(p(StandardView::Top, [1.0, 0.0, 0.0])[0] > 0.0 && p(StandardView::Top, [0.0, 0.0, 1.0])[1] > 0.0);
        // The isometric view sees three faces: x, y and z extremes all nearer than their opposites.
        let iso = |q: [f32; 3]| depth(StandardView::Isometric, q);
        assert!(iso([0.0, 1.0, 0.0]) < iso([0.0, -1.0, 0.0]));
    }

    #[test]
    fn standard_view_and_field_commands_change_only_what_they_name() {
        let s = scene(vec![0.0], None);
        let mut v = ViewState::new(&s);
        let (zoom, mesh) = (v.zoom, v.show_mesh);
        v.apply(Command::StandardView(StandardView::Top), &s);
        assert_eq!((v.yaw, v.pitch), StandardView::Top.angles());
        assert_eq!((v.zoom, v.show_mesh), (zoom, mesh));
        v.apply(Command::SetField(7), &s);
        assert_eq!(v.field, 0, "an index past the fields falls back to the primary scalar");
        assert_eq!(StandardView::ALL.len(), 7);
    }
}
